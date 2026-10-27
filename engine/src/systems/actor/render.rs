//! Actors as the client draws them: each wears its model's skin as a scene, played through the one
//! animation graph its skeleton shares with every body on it, resolving the clip of what it does in
//! the stance its hands show as the server did; it turns the short way round to face where it
//! faces, sounds its clip's cues as they pass, and hangs from each hand the prop of what the hand
//! shows.

use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use bevy::animation::graph::AnimationNodeIndex;
use bevy::animation::{AnimationTargetId, RepeatAnimation};
use bevy::prelude::*;
use bevy::world_serialization::WorldInstanceReady;

use super::layering::{self, Layering};
use super::model::{ClipSource, Models};
use super::skeleton::{Cue, GroupName, Hand, LocalOffset, LocalTurn, SkeletonId};
use super::stance::{self, HandIndex, PropHand};
use super::wearing::{self, Wearing};
use super::{Action, Actor, ActorModel, ModelId, Motion, Playing, Pose};
use crate::core::audio::playback::{PlaySfx, SfxPlace};
use crate::core::content::Content;
use crate::core::content::gltf::NodeTree;
use crate::core::content::media::GltfInfos;
use crate::core::geometry::Radians;
use crate::core::interpolate::{Interpolate, InterpolatePlugin};
use crate::core::math::Pos;
use crate::core::render::transition::WorldViewSystems;
use crate::core::render::world::layers::Layered;
use crate::core::render::world::mannequin::animate_rig;
use crate::core::render::{Animator, Previous, facing_rotation, ground_height, world_point};
use crate::core::time::{Seconds, UnixMillis};
use crate::core::units::Meters;
use crate::core::vfx::cover::Treads;
use crate::core::vfx::render::SiteExtent;
use crate::systems::area::{self, Area};
use crate::systems::item::{Grip, ItemId, Wear};
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

/// One node per clip a body of the skeleton can play, whole, and kept to or kept off each group a
/// set plays a clip on alone.
pub struct SkeletonGraph {
    graph: Handle<AnimationGraph>,
    whole: HashMap<Source, AnimationNodeIndex>,
    only: HashMap<(Source, GroupName), AnimationNodeIndex>,
    outside: HashMap<(Source, GroupName), AnimationNodeIndex>,
}

/// A clip as a graph node finds it: in a library of the skeleton, or in a model's own glb.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Source {
    Library { library: usize, index: usize },
    Model { model: ModelId, index: usize },
}

impl Source {
    fn of(model: ModelId, source: ClipSource) -> Source {
        match source {
            ClipSource::Library { library, index } => Source::Library { library, index },
            ClipSource::Model { index } => Source::Model { model, index },
        }
    }
}

/// The rig an actor wears: its model's scene, the graph of its skeleton, the joints by name once the
/// scene stands, what plays and how far each clip has faded in, and the props its hands hold.
#[derive(Component)]
pub struct Puppet {
    model: ModelId,
    graph: Arc<SkeletonGraph>,
    player: Option<Entity>,
    joints: HashMap<String, Entity>,
    playing: Option<Play>,
    tracks: Vec<Track>,
    turn: Turn,
    held: BTreeMap<HandIndex, (ItemId, Entity)>,
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
    /// The joint of the rig named `name`, once the scene stands.
    pub fn joint(&self, name: &str) -> Option<Entity> {
        self.joints.get(name).copied()
    }

    /// The props the hands hold, by hand.
    pub fn held(&self) -> impl Iterator<Item = (HandIndex, ItemId, Entity)> + '_ {
        self.held
            .iter()
            .map(|(&hand, &(item, prop))| (hand, item, prop))
    }
}

/// Puts a rig on every actor of the shown area that lacks one, or wears another model's: its
/// model's skin, played through its skeleton's graph.
pub fn dress(world: &mut World) {
    let content = world.resource::<Content>().clone();
    let now = Seconds(world.resource::<Time>().elapsed_secs());
    let undressed: Vec<(Entity, ModelId, Radians)> = world
        .query::<(Entity, &Actor, Option<&Puppet>)>()
        .iter(world)
        .filter(|(_, actor, puppet)| puppet.is_none_or(|puppet| puppet.model != actor.model))
        .map(|(entity, actor, _)| (entity, actor.model, actor.facing))
        .collect();
    for (entity, model, facing) in undressed {
        let actor_model = ActorModel::get(&content, model);
        let graph = skeleton_graph(world, &content, actor_model.skeleton_id());
        let hitbox = actor_model.hitbox();
        let assets = world.resource::<AssetServer>().clone();
        let scene = assets.load(GltfAssetLabel::Scene(0).from_asset(actor_model.model().path()));
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
            WorldAssetRoot(scene),
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
                model,
                graph,
                player: None,
                joints: HashMap::new(),
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
    let assets = world.resource::<AssetServer>().clone();
    let mut graph = AnimationGraph::new();
    let groups: Vec<&GroupName> = built.def.groups.keys().collect();
    let rest_bit = groups.len() as u32;
    let all_bits = (1u64 << (rest_bit + 1)) - 1;
    for (bit, target) in mask_groups(&built.rig, &built.def.groups) {
        graph.add_target_to_mask_group(target, bit.unwrap_or(rest_bit));
    }
    let mut sources: Vec<(Source, Handle<AnimationClip>)> = built
        .clips
        .values()
        .map(|clip| {
            let file = built.def.libraries[clip.library].file.path();
            (
                Source::Library {
                    library: clip.library,
                    index: clip.index,
                },
                assets.load(GltfAssetLabel::Animation(clip.index).from_asset(file)),
            )
        })
        .collect();
    for (model, worn) in content.built::<Models>().0.iter().enumerate() {
        let Some(worn) = worn.as_ref().filter(|worn| worn.skeleton_id() == skeleton) else {
            continue;
        };
        let model = ModelId::new(model as u32);
        for (_, index) in worn.own_clips() {
            let file = worn.model().path();
            sources.push((
                Source::Model { model, index },
                assets.load(GltfAssetLabel::Animation(index).from_asset(file)),
            ));
        }
    }
    let grouped: Vec<&GroupName> = built
        .def
        .sets()
        .flat_map(|(_, set)| set.entries().filter_map(|(_, clip)| clip.only()))
        .collect();
    let mut whole = HashMap::new();
    let mut only = HashMap::new();
    let mut outside = HashMap::new();
    for (source, clip) in sources {
        whole.insert(source, graph.add_clip(clip.clone(), 1.0, graph.root));
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

/// A rig's scene, once spawned, hands its animation player the skeleton's graph, its joints are
/// known by name, and the skeleton's layers take them.
pub fn adopt_player(
    ready: On<WorldInstanceReady>,
    content: Res<Content>,
    children: Query<&Children>,
    names: Query<&Name>,
    mut puppets: Query<&mut Puppet>,
    mut commands: Commands,
) {
    let Ok(mut puppet) = puppets.get_mut(ready.entity) else {
        return;
    };
    let Some(player) = animate_rig(ready.entity, &children, &names, &mut commands) else {
        return;
    };
    commands
        .entity(player)
        .insert(AnimationGraphHandle(puppet.graph.graph.clone()));
    puppet.joints = children
        .iter_descendants(ready.entity)
        .filter_map(|entity| Some((names.get(entity).ok()?.as_str().to_owned(), entity)))
        .collect();
    puppet.player = Some(player);
    puppet.playing = None;
    puppet.tracks.clear();
    let model = ActorModel::get(&content, puppet.model);
    match layering::layered(model, &puppet.joints) {
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
    let shown: Vec<(Entity, RenderActor, RenderPosition, Actor, Option<Playing>)> = world
        .query::<(
            Entity,
            &RenderActor,
            &RenderPosition,
            &Actor,
            Option<&Playing>,
        )>()
        .iter(world)
        .map(|(entity, shown, at, actor, playing)| {
            (entity, *shown, *at, actor.clone(), playing.cloned())
        })
        .collect();
    let mut sounds = Vec::new();
    for (entity, shown, at, actor, playing) in shown {
        let model = ActorModel::get(&content, actor.model);
        let stance = wearing::stance_of(world, entity);
        let ground = ground_height(world, at.0);
        let motion = playing
            .as_ref()
            .filter(|playing| playing.motion.action() == shown.action)
            .map_or(Motion::Action(shown.action), |playing| {
                playing.motion.clone()
            });
        let play = Play {
            motion: motion.clone(),
            pose: actor.pose,
            stance: stance.map(|stance| stance.raw()),
            started: playing
                .as_ref()
                .filter(|playing| playing.motion.action() == shown.action)
                .map(|playing| playing.started),
        };
        let resolved = model.resolve(stance, &motion, actor.pose);
        let repeat = if motion.once() {
            RepeatAnimation::Never
        } else {
            RepeatAnimation::Forever
        };
        let rate = model.rate(&motion, &resolved, actor.attack_rate, actor.speed);
        let state = {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            play.hash(&mut hasher);
            hasher.finish()
        };
        let playhead = world.resource_mut::<Animator>().advance(entity, state, now);
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
        if let Ok((mut layered, layering)) = world
            .query::<(&mut Layered, &Layering)>()
            .get_mut(world, entity)
        {
            layering::drive(&mut layered, layering, shown.action, stepped);
        }
        let resting = resolved
            .only
            .map(|_| model.resolve(stance, &Motion::Action(Action::Idle), actor.pose));
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
            let node = |source: Option<ClipSource>, kept: Option<(&GroupName, bool)>| {
                let source = Source::of(actor.model, source?);
                match kept {
                    None => graph.whole.get(&source).copied(),
                    Some((group, true)) => graph.only.get(&(source, group.clone())).copied(),
                    Some((group, false)) => graph.outside.get(&(source, group.clone())).copied(),
                }
            };
            match (resolved.only, resting) {
                (Some(group), Some(resting)) => [
                    node(resting.source, Some((group, false))),
                    node(resolved.source, Some((group, true))),
                ]
                .into_iter()
                .flatten()
                .collect(),
                _ => node(resolved.source, None).into_iter().collect(),
            }
        };
        let restart = puppet.playing.as_ref() != Some(&play);
        if restart {
            puppet.playing = Some(play);
        }
        let player = puppet.player;
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
        .query::<(Entity, &Actor, &Puppet, Option<&Wearing>)>()
        .iter(world)
        .filter(|(_, _, puppet, _)| puppet.player.is_some())
        .map(|(entity, actor, _, wearing)| {
            (
                entity,
                actor.model,
                wearing
                    .map(|wearing| wearing.hands.clone())
                    .unwrap_or_default(),
            )
        })
        .collect();
    for (entity, model, shown) in dressed {
        let model = ActorModel::get(&content, model);
        let hands = model.hands();
        let wanted = hung(&content, model, &shown);
        let Some(puppet) = world.get::<Puppet>(entity) else {
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
                let joint = puppet.joint(&hands[hand.index()].joint.0)?;
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
            let Some(Wear::Hand { grip, .. }) = def.wear() else {
                continue;
            };
            let look = def.look.get(&content);
            let placed = held_at(&content, model, &hands[hand.index()], grip);
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
/// hand and the grip say, and as big as it lies on the ground.
pub fn held_at(content: &Content, model: &ActorModel, hand: &Hand, grip: &Grip) -> Transform {
    let rest = rest_scale(content, model, hand);
    Transform::from_scale(Vec3::splat(1.0 / rest))
        * hold(&hand.offset, &hand.turn)
        * hold(&grip.offset, &grip.turn)
}

/// A move and turn in a joint's frame: the turn's degrees about x, y and z in that order.
fn hold(offset: &LocalOffset, turn: &LocalTurn) -> Transform {
    let [x, y, z] = turn.array().map(f32::to_radians);
    Transform::from_translation(Vec3::from(offset.array())).with_rotation(Quat::from_euler(
        EulerRot::XYZ,
        x,
        y,
        z,
    ))
}

/// How much the model's glb scales the hand's joint at rest, which a prop held there undoes so it
/// keeps the size it has lying on the ground.
fn rest_scale(content: &Content, model: &ActorModel, hand: &Hand) -> f32 {
    let Some(info) = content.built::<GltfInfos>().0.get(model.model().as_str()) else {
        return 1.0;
    };
    let tree = &info.tree;
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
