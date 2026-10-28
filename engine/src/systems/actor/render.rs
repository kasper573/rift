//! Actors as the client draws them: each stands its model's body, its parts' skins over its
//! skeleton's rig, played through the one animation graph its skeleton shares with every body on it, resolving the clip of what it does in
//! the stance its hands show as the server did; it turns the short way round to face where it
//! faces, sounds its clip's cues as they pass, and hangs from each hand the prop of what the hand
//! shows.

use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use bevy::animation::graph::AnimationNodeIndex;
use bevy::animation::{AnimationTargetId, RepeatAnimation};
use bevy::prelude::*;

use super::appearance::look;
use super::held::{self, Held};
use super::layering::{self, Layering};
use super::rigging::BuiltSkeleton;
use super::skeleton::{Cue, GroupName, Hand, LocalOffset, LocalTurn, SkeletonId};
use super::skeleton::{EquipmentSlot, GestureName};
use super::stance::{self, HandIndex, PropHand};
use super::{Action, Actor, ActorModel, Appearance, Hurt, ModelId, Motion, Playing, Pose};
use crate::core::audio::playback::{PlaySfx, SfxPlace};
use crate::core::content::Content;
use crate::core::content::gltf::NodeTree;
use crate::core::geometry::Radians;
use crate::core::interpolate::{Interpolate, InterpolatePlugin};
use crate::core::math::{Pos, Rng};
use crate::core::render::transition::WorldViewSystems;
use crate::core::render::world::body::{self, Body, RigReady};
use crate::core::render::world::layers::Layered;
use crate::core::render::world::rig_clip;
use crate::core::render::{Animator, Previous, Turns, facing_rotation, ground_height, world_point};
use crate::core::text::Typewriter;
use crate::core::time::{Seconds, UnixMillis};
use crate::core::units::Meters;
use crate::core::vfx::cover::Treads;
use crate::core::vfx::render::SiteExtent;
use crate::systems::area::{self, Area};
use crate::systems::equipment::{self, Equipment};
use crate::systems::item::{Grip, ItemId, PropAxis, Wear};
use crate::systems::movement::RenderPosition;
use crate::systems::screen::ClientScreen;

/// How long an actor takes to turn to a new facing.
const TURN: Seconds = Seconds(0.12);
/// How long one clip blends into the next.
const CROSSFADE: Seconds = Seconds(0.1);

pub struct ActorRenderPlugin;

impl Plugin for ActorRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(InterpolatePlugin::<RenderActor>::default())
            .add_observer(adopt_player)
            .add_systems(
                Update,
                (dress, pose, hang)
                    .chain()
                    .in_set(WorldViewSystems)
                    .run_if(in_state(ClientScreen::Area)),
            );
    }
}

/// The facing and animation to render an actor with. The server recomputes these alongside its
/// movement every tick, so they ride the same replication cadence as `RenderPosition`: the legs go
/// idle, and the heading turns, as the on-screen motion stops or changes course — not the snapshot
/// earlier they would if read live, which reads as the actor sliding or drifting. Discrete values,
/// so they switch as their segment starts rather than blending. The client's own character, walked
/// ahead of the snapshots, takes them from its own walk.
#[derive(Component, Clone, Copy, PartialEq)]
pub struct RenderActor {
    pub facing: Radians,
    pub action: Action,
}

impl Interpolate for RenderActor {
    type Source = Actor;

    fn sample(actor: &Actor) -> RenderActor {
        RenderActor {
            facing: actor.facing,
            action: actor.action,
        }
    }
}

/// The graph every body of one skeleton plays through, built when the first of them dresses.
#[derive(Resource, Default)]
struct SkeletonGraphs(HashMap<SkeletonId, Arc<SkeletonGraph>>);

/// One node per clip a body of the skeleton can play, by the clip's place among the skeleton's,
/// whole, and kept to or kept off each group a set plays a clip on alone.
pub struct SkeletonGraph {
    graph: Handle<AnimationGraph>,
    whole: Vec<AnimationNodeIndex>,
    only: HashMap<(usize, GroupName), AnimationNodeIndex>,
    outside: HashMap<(usize, GroupName), AnimationNodeIndex>,
}

/// Every clip made for a skeleton's rig so far, each made once from its file's keyframes: by the
/// skeleton, which the entry holds so no other is made where it was, and the clip's place among
/// its clips.
#[derive(Resource, Default)]
struct RigClips(HashMap<usize, MadeClips>);

/// The clips made for one skeleton's rig, by their place among its clips.
struct MadeClips {
    skeleton: Arc<BuiltSkeleton>,
    clips: HashMap<usize, Handle<AnimationClip>>,
}

/// The clip at `played` among `skeleton`'s, made for its rig the first time it is asked for: its
/// own keyframes on the joints of ours they name, or a source rig's played through its pairs.
pub fn rig_clip(
    world: &mut World,
    skeleton: &Arc<BuiltSkeleton>,
    played: usize,
) -> Handle<AnimationClip> {
    let key = Arc::as_ptr(skeleton) as usize;
    let mut clips = world.get_resource_or_init::<RigClips>();
    clips.0.retain(|_, made| {
        Arc::strong_count(&made.skeleton) > 1 || Arc::ptr_eq(&made.skeleton, skeleton)
    });
    if let Some(handle) = clips.0.get(&key).and_then(|made| made.clips.get(&played)) {
        return handle.clone();
    }
    let clip = &skeleton.clips[played];
    let pairs = clip
        .source
        .map(|source| skeleton.def.sources[source].joint_pairs());
    let handle = rig_clip::rig_clip(
        world,
        skeleton.rig.clone(),
        clip.clip.file().as_str(),
        clip.animation,
        rig_clip::maker(skeleton.rig.clone(), pairs),
    );
    world
        .resource_mut::<RigClips>()
        .0
        .entry(key)
        .or_insert_with(|| MadeClips {
            skeleton: skeleton.clone(),
            clips: HashMap::new(),
        })
        .clips
        .insert(played, handle.clone());
    handle
}

/// The body an actor wears: the appearance and worn items it was dressed in, the graph of its
/// skeleton, what plays and how far each clip has faded in, and the props its hands hold.
#[derive(Component)]
pub struct Puppet {
    dressed: Dressed,
    graph: Arc<SkeletonGraph>,
    playing: Option<Play>,
    tracks: Vec<Track>,
    turn: Turn,
    held: BTreeMap<HandIndex, (ItemId, Entity)>,
}

/// What a body was last dressed as: its appearance, and the items worn in its slots.
#[derive(Clone, PartialEq)]
struct Dressed {
    appearance: Appearance,
    worn: BTreeMap<EquipmentSlot, ItemId>,
}

/// A prop hanging from a hand of an actor's rig.
#[derive(Component)]
pub struct HeldProp;

/// What plays, from when: an attack struck again restarts the same clip.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Play {
    motion: Motion,
    pose: Pose,
    stance: Option<u32>,
    started: Option<UnixMillis>,
    struck: Option<u32>,
    said: Option<Entity>,
    pick: usize,
}

/// The blows a body has seen land, and when it began to flinch at the newest. A blow that landed
/// before the body came into view is never flinched at.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
struct Flinch {
    blows: u32,
    since: Option<Seconds>,
}

/// A line a body says, on the line being typed: the body talks while the line types, and makes the
/// gesture the line is said with from when it began, when its sets name one.
#[derive(Component, Clone, Debug)]
pub struct Said {
    pub by: Entity,
    pub gesture: Option<GestureName>,
    pub since: Seconds,
}

/// A node playing on the rig, fading toward its weight.
#[derive(Clone, Copy, Debug)]
struct Track {
    node: AnimationNodeIndex,
    weight: f32,
    target: f32,
}

#[derive(Clone, Copy, Debug)]
struct Turn {
    from: Radians,
    to: Radians,
    started: Seconds,
}

impl Turn {
    fn shown(&self, now: Seconds) -> Radians {
        let t = ((now - self.started).0 / TURN.0).clamp(0.0, 1.0);
        let apart = (self.to.0 - self.from.0).rem_euclid(std::f32::consts::TAU);
        let short = if apart > std::f32::consts::PI {
            apart - std::f32::consts::TAU
        } else {
            apart
        };
        Radians(self.from.0 + short * t)
    }
}

impl Puppet {
    /// Which of `motion`'s cell's clips the body shows, while it shows that motion.
    pub fn pick_of(&self, motion: &Motion) -> usize {
        self.playing
            .as_ref()
            .filter(|play| play.motion == *motion)
            .map_or(0, |play| play.pick)
    }

    /// What the body shows playing.
    pub fn shown(&self) -> Option<&Motion> {
        self.playing.as_ref().map(|play| &play.motion)
    }

    /// The props the hands hold, by hand.
    pub fn held(&self) -> impl Iterator<Item = (HandIndex, ItemId, Entity)> + '_ {
        self.held
            .iter()
            .map(|(&hand, &(item, prop))| (hand, item, prop))
    }
}

/// Stands a body on every actor of the shown area that lacks one, or wears another model's: its
/// look over its skeleton's rig, played through its skeleton's graph. A body whose appearance or
/// worn items change keeps its rig and swaps its skins for those of its new look.
pub fn dress(world: &mut World) {
    let content = world.resource::<Content>().clone();
    let now = Seconds(world.resource::<Time>().elapsed_secs());
    let changed: Vec<(Entity, Dressed, Radians, bool)> = world
        .query::<(
            Entity,
            &Actor,
            &Appearance,
            Option<&Equipment>,
            Option<&Puppet>,
        )>()
        .iter(world)
        .filter_map(|(entity, actor, appearance, equipment, puppet)| {
            let worn = || equipment.into_iter().flat_map(equipment::worn_slots);
            let same_model =
                puppet.is_some_and(|puppet| puppet.dressed.appearance.model == appearance.model);
            let same_look = puppet.is_some_and(|puppet| {
                puppet.dressed.appearance == *appearance && puppet.dressed.worn.iter().eq(worn())
            });
            (!same_look).then(|| {
                let dressed = Dressed {
                    appearance: appearance.clone(),
                    worn: worn().map(|(slot, item)| (slot.clone(), *item)).collect(),
                };
                (entity, dressed, actor.facing, !same_model)
            })
        })
        .collect();
    for (entity, dressed, facing, fresh) in changed {
        let actor_model = ActorModel::get(&content, dressed.appearance.model);
        let files = actor_model.dressed(
            &content,
            &look(&content, &dressed.appearance, &dressed.worn),
        );
        if !fresh {
            body::reskin(world, entity, &files.skins);
            if let Some(mut puppet) = world.get_mut::<Puppet>(entity) {
                puppet.dressed = dressed;
            }
            continue;
        }
        let graph = skeleton_graph(world, &content, actor_model.skeleton_id());
        let hitbox = actor_model.hitbox();
        let held: Vec<Entity> = world
            .get::<Puppet>(entity)
            .map(|puppet| puppet.held.values().map(|(_, prop)| *prop).collect())
            .unwrap_or_default();
        for prop in held {
            if let Ok(prop) = world.get_entity_mut(prop) {
                prop.despawn();
            }
        }
        world.entity_mut(entity).insert((
            Transform::default(),
            Visibility::default(),
            SiteExtent {
                footprint: Vec2::splat(hitbox.radius.0 * 2.0),
                height: hitbox.height.0,
            },
            Treads {
                radius: if actor_model.airborne() {
                    0.0
                } else {
                    hitbox.radius.0 * 2.0
                },
            },
            Puppet {
                dressed,
                graph,
                playing: None,
                tracks: Vec::new(),
                turn: Turn {
                    from: facing,
                    to: facing,
                    started: now,
                },
                held: BTreeMap::new(),
            },
        ));
        body::dress(world, entity, &files);
    }
}

/// The graph of `skeleton`, built the first time a body on it dresses.
fn skeleton_graph(
    world: &mut World,
    content: &Content,
    skeleton: SkeletonId,
) -> Arc<SkeletonGraph> {
    if let Some(graph) = world
        .get_resource_or_init::<SkeletonGraphs>()
        .0
        .get(&skeleton)
    {
        return graph.clone();
    }
    let built = super::model::Skeletons::loaded(content, skeleton)
        .cloned()
        .unwrap_or_else(|| panic!("skeleton {} failed to load", skeleton.path(content)));
    let mut graph = AnimationGraph::new();
    let groups: Vec<&GroupName> = built.def.groups.keys().collect();
    let rest_bit = groups.len() as u32;
    let all_bits = (1u64 << (rest_bit + 1)) - 1;
    for (bit, target) in mask_groups(&built.rig, &built.def.groups) {
        graph.add_target_to_mask_group(target, bit.unwrap_or(rest_bit));
    }
    let sources: Vec<(usize, Handle<AnimationClip>)> = (0..built.clips.len())
        .map(|played| (played, rig_clip(world, &built, played)))
        .collect();
    let grouped: Vec<&GroupName> = built
        .def
        .sets()
        .flat_map(|(_, set)| set.entries().filter_map(|(_, clip)| clip.only()))
        .collect();
    let mut whole = Vec::new();
    let mut only = HashMap::new();
    let mut outside = HashMap::new();
    for (source, clip) in sources {
        whole.push(graph.add_clip(clip.clone(), 1.0, graph.root));
        for &group in &grouped {
            let Some(bit) = groups.iter().position(|known| *known == group) else {
                continue;
            };
            let bit = 1u64 << bit;
            let key = (source, group.clone());
            if only.contains_key(&key) {
                continue;
            }
            only.insert(
                key.clone(),
                graph.add_clip_with_mask(clip.clone(), all_bits & !bit, 1.0, graph.root),
            );
            outside.insert(
                key,
                graph.add_clip_with_mask(clip.clone(), bit, 1.0, graph.root),
            );
        }
    }
    let graph = Arc::new(SkeletonGraph {
        graph: world.resource_mut::<Assets<AnimationGraph>>().add(graph),
        whole,
        only,
        outside,
    });
    world
        .resource_mut::<SkeletonGraphs>()
        .0
        .insert(skeleton, graph.clone());
    graph
}

/// Every joint of `rig` as a clip targets it, with the group whose root it lies under, if any.
fn mask_groups(
    rig: &NodeTree,
    groups: &BTreeMap<GroupName, Vec<super::skeleton::JointName>>,
) -> Vec<(Option<u32>, AnimationTargetId)> {
    let roots: Vec<(u32, usize)> = groups
        .values()
        .enumerate()
        .flat_map(|(bit, roots)| {
            roots.iter().filter_map(move |root| {
                let at = (0..rig.nodes.len()).find(|&at| rig.nodes[at].name == root.0)?;
                Some((bit as u32, at))
            })
        })
        .collect();
    rig.joints()
        .map(|joint| {
            let group = roots
                .iter()
                .find(|(_, root)| rig.descends(joint, *root))
                .map(|(bit, _)| *bit);
            (group, AnimationTargetId::from_iter(rig.path(joint)))
        })
        .collect()
}

/// A body's rig, once it stands, plays through the skeleton's graph, and the skeleton's layers take
/// its joints.
pub fn adopt_player(
    ready: On<RigReady>,
    content: Res<Content>,
    mut puppets: Query<(&mut Puppet, &Body)>,
    mut commands: Commands,
) {
    let Ok((mut puppet, body)) = puppets.get_mut(ready.entity) else {
        return;
    };
    let Some(player) = body.player() else {
        return;
    };
    commands
        .entity(player)
        .insert(AnimationGraphHandle(puppet.graph.graph.clone()));
    puppet.playing = None;
    puppet.tracks.clear();
    let model = ActorModel::get(&content, puppet.dressed.appearance.model);
    match layering::layered(model, body.joints()) {
        Some(layers) => commands.entity(ready.entity).insert(layers),
        None => commands
            .entity(ready.entity)
            .remove::<(Layered, Layering)>(),
    };
}

/// Places every rig where its actor shows, turned the way it faces, playing the clip its stance
/// resolves for what it does, and sounds the cues its clip crosses.
pub fn pose(world: &mut World) {
    let content = world.resource::<Content>().clone();
    let now = Seconds(world.resource::<Time>().elapsed_secs());
    let delta = Seconds(world.resource::<Time>().delta_secs());
    let shown: Vec<(
        Entity,
        RenderActor,
        RenderPosition,
        Actor,
        ModelId,
        Option<Playing>,
    )> = world
        .query::<(
            Entity,
            &RenderActor,
            &RenderPosition,
            &Actor,
            &Appearance,
            Option<&Playing>,
        )>()
        .iter(world)
        .map(|(entity, shown, at, actor, appearance, playing)| {
            (
                entity,
                *shown,
                *at,
                actor.clone(),
                appearance.model,
                playing.cloned(),
            )
        })
        .collect();
    let saying = saying(world);
    let mut sounds = Vec::new();
    for (entity, shown, at, actor, model, playing) in shown {
        let model = ActorModel::get(&content, model);
        let stance = held::stance_of(world, entity);
        let ground = ground_height(world, at.0);
        let given = playing
            .as_ref()
            .filter(|playing| playing.motion.shows_over(shown.action));
        let blows = world.get::<Hurt>(entity).map_or(0, |hurt| hurt.0);
        let flinched = flinched(world, entity, blows, now);
        let flinch = Motion::Action(Action::Hurt);
        let flinch_pick = blows as usize % model.cell(stance, &flinch, actor.pose).clips().len();
        let flinching = given.is_none()
            && matches!(shown.action, Action::Idle | Action::Talk)
            && model.names(stance, Action::Hurt)
            && flinched.is_some_and(|since| {
                now - since
                    < model
                        .resolve(stance, &flinch, actor.pose, flinch_pick)
                        .length
            });
        let speech = saying.get(&entity);
        let gesture = speech
            .and_then(|speech| speech.gesture.as_ref())
            .filter(|_| given.is_none() && !flinching)
            .filter(|_| matches!(shown.action, Action::Idle | Action::Talk))
            .and_then(|(line, gesture, since)| {
                let gesture = Motion::Gesture(gesture.clone());
                let pick = line.index_u32() as usize
                    % model.cell(stance, &gesture, actor.pose).clips().len();
                let length = model.resolve(stance, &gesture, actor.pose, pick).length;
                (model.plays(stance, &gesture) && now - *since < length)
                    .then_some((*line, gesture, pick))
            });
        let talks = speech.is_some_and(|speech| speech.talking);
        let motion = match (given, &gesture) {
            _ if flinching => flinch,
            (Some(playing), _) => playing.motion.clone(),
            (None, Some((_, gesture, _))) => gesture.clone(),
            (None, None) if shown.action == Action::Talk && !talks => Motion::Action(Action::Idle),
            (None, None) => Motion::Action(shown.action),
        };
        let mut play = Play {
            motion: motion.clone(),
            pose: actor.pose,
            stance: stance.map(|stance| stance.raw()),
            started: given.map(|playing| playing.started),
            struck: flinching.then_some(blows),
            said: gesture.as_ref().map(|(line, ..)| *line),
            pick: 0,
        };
        let repeat = if motion.once() {
            RepeatAnimation::Never
        } else {
            RepeatAnimation::Forever
        };
        let state = {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            play.hash(&mut hasher);
            hasher.finish()
        };
        let cell = model.cell(stance, &motion, actor.pose);
        let played_once = |pick| {
            let resolved = model.resolve(stance, &motion, actor.pose, pick);
            resolved.length / model.rate(&motion, &resolved, actor.attack_rate, actor.speed)
        };
        let mut next = |last, rng: &mut Rng| cell.after(last, rng);
        let turns = (!motion.once() && cell.clips().len() > 1).then_some(Turns {
            length: &played_once,
            next: &mut next,
        });
        let playhead = world
            .resource_mut::<Animator>()
            .advance(entity, state, now, turns);
        play.pick = match (given, &gesture) {
            _ if flinching => flinch_pick,
            (Some(playing), _) => playing.pick,
            (None, Some((_, _, pick))) => *pick,
            (None, None) => playhead.turn,
        };
        let resolved = model.resolve(stance, &motion, actor.pose, play.pick);
        let rate = model.rate(&motion, &resolved, actor.attack_rate, actor.speed);
        let stepped = {
            let prev = match playhead.previous {
                Previous::Continued(played) => Some(played),
                Previous::Unseen | Previous::Restarted => None,
            };
            let crossed = ActorModel::cues(&resolved, &motion, prev, playhead.elapsed, rate);
            let (sfx, stepped) = cue_sounds(&crossed, area::of(world, entity), at.0);
            sounds.extend(sfx);
            stepped
        };
        let holding: Vec<HandIndex> = world.get::<Puppet>(entity).map_or_else(Vec::new, |puppet| {
            puppet.held().map(|(hand, ..)| hand).collect()
        });
        if let Ok((mut layered, layering)) = world
            .query::<(&mut Layered, &Layering)>()
            .get_mut(world, entity)
        {
            layering::drive(&mut layered, layering, shown.action, &holding, stepped);
        }
        let resting = resolved
            .only
            .map(|_| model.resolve(stance, &Motion::Action(Action::Idle), actor.pose, 0));
        let Some(mut puppet) = world.get_mut::<Puppet>(entity) else {
            continue;
        };
        if puppet.turn.to != shown.facing {
            puppet.turn = Turn {
                from: puppet.turn.shown(now),
                to: shown.facing,
                started: now,
            };
        }
        let facing = puppet.turn.shown(now);
        let wanted: Vec<AnimationNodeIndex> = {
            let graph = &puppet.graph;
            let node = |source: usize, kept: Option<(&GroupName, bool)>| match kept {
                None => graph.whole.get(source).copied(),
                Some((group, true)) => graph.only.get(&(source, group.clone())).copied(),
                Some((group, false)) => graph.outside.get(&(source, group.clone())).copied(),
            };
            match (resolved.only, resting) {
                (Some(group), Some(resting)) => [
                    node(resting.played, Some((group, false))),
                    node(resolved.played, Some((group, true))),
                ]
                .into_iter()
                .flatten()
                .collect(),
                _ => node(resolved.played, None).into_iter().collect(),
            }
        };
        let restart = puppet.playing.as_ref() != Some(&play);
        if restart {
            puppet.playing = Some(play);
        }
        let player = world.get::<Body>(entity).and_then(Body::player);
        let scale = model.scale();
        if let Some(mut transform) = world.get_mut::<Transform>(entity) {
            transform.translation = world_point(at.0, ground);
            transform.rotation = facing_rotation(facing);
            transform.scale = Vec3::splat(scale);
        }
        let Some(player) = player else {
            continue;
        };
        let Some(mut tracks) = world
            .get_mut::<Puppet>(entity)
            .map(|mut puppet| std::mem::take(&mut puppet.tracks))
        else {
            continue;
        };
        if let Some(mut animation) = world.get_mut::<AnimationPlayer>(player) {
            if restart {
                for track in &mut tracks {
                    track.target = 0.0;
                }
                for &node in &wanted {
                    let fresh = !tracks.iter().any(|track| track.node == node);
                    let active = animation.start(node);
                    active.set_repeat(repeat).set_speed(rate.0);
                    if fresh {
                        active.set_weight(0.0);
                        tracks.push(Track {
                            node,
                            weight: 0.0,
                            target: 1.0,
                        });
                    }
                }
                for track in &mut tracks {
                    if wanted.contains(&track.node) {
                        track.target = 1.0;
                    }
                }
            }
            for track in &mut tracks {
                let step = if CROSSFADE.0 > 0.0 {
                    delta.0 / CROSSFADE.0
                } else {
                    1.0
                };
                track.weight = if track.target > track.weight {
                    (track.weight + step).min(track.target)
                } else {
                    (track.weight - step).max(track.target)
                };
                if let Some(active) = animation.animation_mut(track.node) {
                    active.set_weight(track.weight);
                    if wanted.contains(&track.node) {
                        active.set_speed(rate.0);
                    }
                }
            }
            for track in tracks
                .iter()
                .filter(|track| track.weight <= 0.0 && track.target <= 0.0)
            {
                animation.stop(track.node);
            }
            tracks.retain(|track| track.weight > 0.0 || track.target > 0.0);
        }
        if let Some(mut puppet) = world.get_mut::<Puppet>(entity) {
            puppet.tracks = tracks;
        }
    }
    for sound in sounds {
        world.write_message(sound);
    }
    world.resource_scope(|world, mut animator: Mut<Animator>| {
        animator.retain(|entity| world.get_entity(entity).is_ok());
    });
}

/// What each body says: whether a line it says still types, and the newest line it said with a
/// gesture, by the line, and when it began.
struct Saying {
    talking: bool,
    gesture: Option<(Entity, GestureName, Seconds)>,
}

fn saying(world: &mut World) -> HashMap<Entity, Saying> {
    let mut saying: HashMap<Entity, Saying> = HashMap::new();
    for (line, said, typewriter) in world.query::<(Entity, &Said, &Typewriter)>().iter(world) {
        let body = saying.entry(said.by).or_insert(Saying {
            talking: false,
            gesture: None,
        });
        body.talking |= !typewriter.is_finished();
        if let Some(gesture) = &said.gesture
            && body
                .gesture
                .as_ref()
                .is_none_or(|(_, _, since)| *since < said.since)
        {
            body.gesture = Some((line, gesture.clone(), said.since));
        }
    }
    saying
}

/// When the body began to flinch at the newest blow it has seen land, keeping count of the blows.
fn flinched(world: &mut World, entity: Entity, blows: u32, now: Seconds) -> Option<Seconds> {
    let seen = world.get::<Flinch>(entity).copied();
    let flinch = match seen {
        Some(seen) if seen.blows == blows => return seen.since,
        Some(_) => Flinch {
            blows,
            since: Some(now),
        },
        None => Flinch { blows, since: None },
    };
    world.entity_mut(entity).insert(flinch);
    flinch.since
}

/// What a body standing at `at` on `ground` sounds for the cues of its clip it just `crossed`: each
/// cue's own sound, and the footstep of the ground under it when one puts a foot down, which it
/// also says.
pub fn cue_sounds(
    crossed: &[&Cue],
    ground: Option<&Area>,
    at: Pos<Meters>,
) -> (Vec<PlaySfx>, bool) {
    let stepped = crossed.iter().any(|cue| cue.step);
    let footstep = ground
        .filter(|_| stepped)
        .and_then(|ground| ground.footstep_at(at));
    let sounds = crossed
        .iter()
        .filter_map(|cue| cue.sound.as_ref())
        .chain(footstep)
        .map(|id| PlaySfx {
            id: *id,
            place: SfxPlace::World(at),
        })
        .collect();
    (sounds, stepped)
}

/// Hangs from each hand of a dressed rig the prop of the item it shows, moved by the hand's and the
/// item's grip, and takes it down the frame the hand empties. A two-hander hangs from the first or
/// the last hand it takes, as its stance holds it.
pub fn hang(world: &mut World) {
    let content = world.resource::<Content>().clone();
    let dressed: Vec<(Entity, ModelId, BTreeMap<HandIndex, ItemId>)> = world
        .query::<(Entity, &Appearance, &Body, Option<&Held>)>()
        .iter(world)
        .filter(|(_, _, body, _)| body.player().is_some())
        .map(|(entity, appearance, _, held)| {
            (
                entity,
                appearance.model,
                held.map(|held| held.hands.clone()).unwrap_or_default(),
            )
        })
        .collect();
    for (entity, model, shown) in dressed {
        let model = ActorModel::get(&content, model);
        let hands = model.hands();
        let wanted = hung(&content, model, &shown);
        let (Some(puppet), Some(body)) = (world.get::<Puppet>(entity), world.get::<Body>(entity))
        else {
            continue;
        };
        let gone: Vec<(HandIndex, Entity)> = puppet
            .held
            .iter()
            .filter(|(hand, (item, _))| wanted.get(hand) != Some(item))
            .map(|(&hand, &(_, prop))| (hand, prop))
            .collect();
        let new: Vec<(HandIndex, ItemId, Entity)> = wanted
            .iter()
            .filter(|(hand, item)| puppet.held.get(hand).map(|(held, _)| held) != Some(item))
            .filter_map(|(&hand, &item)| {
                let joint = body.joint(&hands[hand.index()].joint.0)?;
                Some((hand, item, joint))
            })
            .collect();
        for (hand, prop) in gone {
            if let Ok(prop) = world.get_entity_mut(prop) {
                prop.despawn();
            }
            if let Some(mut puppet) = world.get_mut::<Puppet>(entity) {
                puppet.held.remove(&hand);
            }
        }
        for (hand, item, joint) in new {
            let def = item.get(&content);
            let Some(Wear::Held { grip, .. }) = def.wear() else {
                continue;
            };
            let look = def.look.get(&content);
            let placed = held_at(model, &hands[hand.index()], grip);
            let scene = world
                .resource::<AssetServer>()
                .load(GltfAssetLabel::Scene(0).from_asset(look.model.path()));
            let prop = world
                .spawn((
                    HeldProp,
                    WorldAssetRoot(scene),
                    placed,
                    Visibility::default(),
                    ChildOf(joint),
                ))
                .id();
            if let Some(mut puppet) = world.get_mut::<Puppet>(entity) {
                puppet.held.insert(hand, (item, prop));
            }
        }
    }
}

/// The hand each item `shown` hangs its prop from on `model`: once per item, from the first or the
/// last hand it takes as its stance holds it, and only from hands the body has.
pub fn hung(
    content: &Content,
    model: &ActorModel,
    shown: &BTreeMap<HandIndex, ItemId>,
) -> BTreeMap<HandIndex, ItemId> {
    shown
        .iter()
        .filter(|&(&hand, &item)| {
            hand.index() < model.hands().len() && hangs_from(content, shown, hand, item)
        })
        .map(|(&hand, &item)| (hand, item))
        .collect()
}

/// Whether `item`'s prop hangs from `hand` among the hands that show it: an item spans as many
/// hands in a row as its stance takes, so the same item in hands beside each other is one item
/// held in both, or two of it held one apiece; it hangs from the first hand of its span, or the
/// last when its stance holds it there.
fn hangs_from(
    content: &Content,
    shown: &BTreeMap<HandIndex, ItemId>,
    hand: HandIndex,
    item: ItemId,
) -> bool {
    let Some(stance) = stance::of_item(content, item) else {
        return true;
    };
    let stance = stance.get(content);
    let span = stance.hands.0.max(1);
    let mut before = 0;
    while hand.0 > before && shown.get(&HandIndex(hand.0 - before - 1)) == Some(&item) {
        before += 1;
    }
    let at = before % span;
    match stance.prop {
        PropHand::First => at == 0,
        PropHand::Last => at == span - 1,
    }
}

/// Where a prop held by `grip` sits under the joint of `hand` on `model`: moved and turned as the
/// hand and the grip say, and as big as it lies on the ground. A mirrored hand holds it as the
/// mirror image of the way the hands grips are fitted to hold it, mirrored again across the axis
/// it is its own mirror image across, which leaves it turned over, never inside out.
pub fn held_at(model: &ActorModel, hand: &Hand, grip: &Grip) -> Transform {
    let rest = rest_scale(model, hand);
    let mut held = grip_frame(hand) * placed(&grip.offset, &grip.turn);
    if hand.mirrored {
        held *= Mat4::from_scale(match grip.symmetry {
            PropAxis::X => Vec3::new(-1.0, 1.0, 1.0),
            PropAxis::Y => Vec3::new(1.0, -1.0, 1.0),
            PropAxis::Z => Vec3::new(1.0, 1.0, -1.0),
        });
    }
    Transform::from_scale(Vec3::splat(1.0 / rest)) * Transform::from_matrix(held)
}

/// The grip frame of `hand` in its joint's frame, its +Z out of the back of the hand whether or
/// not the hand is mirrored.
pub fn grip_frame(hand: &Hand) -> Mat4 {
    let frame = placed(&hand.offset, &hand.turn);
    if hand.mirrored {
        frame * Mat4::from_scale(Vec3::new(1.0, 1.0, -1.0))
    } else {
        frame
    }
}

/// A move and turn in a joint's frame: the turn's degrees about x, y and z in that order.
fn placed(offset: &LocalOffset, turn: &LocalTurn) -> Mat4 {
    let [x, y, z] = turn.array().map(f32::to_radians);
    Mat4::from_rotation_translation(
        Quat::from_euler(EulerRot::XYZ, x, y, z),
        Vec3::from(offset.array()),
    )
}

/// How much the rig scales the hand's joint at rest, which a prop held there undoes so it keeps the
/// size it has lying on the ground.
fn rest_scale(model: &ActorModel, hand: &Hand) -> f32 {
    let tree = &model.skeleton().rig;
    let Some(joint) = tree.named(&hand.joint.0) else {
        return 1.0;
    };
    let mut scale = 1.0;
    let mut at = Some(joint);
    while let Some(node) = at {
        scale *= tree.nodes[node].rest.scale.x;
        at = tree.nodes[node].parent;
    }
    if scale > 0.0 { scale } else { 1.0 }
}
