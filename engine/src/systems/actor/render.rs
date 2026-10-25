//! Actors as the client draws them: each wears its model's rig as a scene, plays the clip of the
//! action the server shows it in, turns the short way round to face where it faces, and sounds its
//! clip's cues as they pass.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::time::Duration;

use bevy::animation::RepeatAnimation;
use bevy::animation::graph::AnimationNodeIndex;
use bevy::animation::transition::AnimationTransitions;
use bevy::prelude::*;
use bevy::world_serialization::WorldInstanceReady;

use super::{Action, Actor, ActorModel, ModelId, Playing, Pose};
use crate::core::audio::playback::{PlaySfx, SfxPlace};
use crate::core::content::Content;
use crate::core::content::media::GltfInfos;
use crate::core::geometry::Radians;
use crate::core::interpolate::{Interpolate, InterpolatePlugin};
use crate::core::render::transition::WorldViewSystems;
use crate::core::render::{Animator, Previous, ground_height, world_point};
use crate::core::time::{Seconds, UnixMillis};
use crate::core::vfx::cover::Treads;
use crate::core::vfx::render::SiteExtent;
use crate::systems::area;
use crate::systems::movement::RenderPosition;
use crate::systems::screen::ClientScreen;

/// How long an actor takes to turn to a new facing.
const TURN: Seconds = Seconds(0.12);
/// How long one clip blends into the next.
const CROSSFADE: Duration = Duration::from_millis(100);

pub struct ActorRenderPlugin;

impl Plugin for ActorRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(InterpolatePlugin::<RenderActor>::default())
            .add_observer(adopt_player)
            .add_systems(
                Update,
                (dress, pose)
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

/// The rig an actor wears: its model's scene, the animation nodes of its clips by name, and where
/// its playback stands.
#[derive(Component)]
pub struct Puppet {
    model: ModelId,
    nodes: HashMap<String, AnimationNodeIndex>,
    graph: Handle<AnimationGraph>,
    player: Option<Entity>,
    playing: Option<Play>,
    turn: Turn,
}

/// What clip plays, from when: an attack struck again restarts the same clip.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Play {
    action: Action,
    pose: Pose,
    started: Option<UnixMillis>,
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

/// Puts a rig on every actor of the shown area that lacks one, or wears another model's.
fn dress(world: &mut World) {
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
        let path = actor_model.model().path();
        let hitbox = actor_model.hitbox();
        let clips = content
            .built::<GltfInfos>()
            .0
            .get(&path)
            .map(|info| {
                info.clip_names()
                    .iter()
                    .map(|name| (*name).to_owned())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let assets = world.resource::<AssetServer>().clone();
        let mut graph = AnimationGraph::new();
        let nodes = clips
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let clip = assets.load(GltfAssetLabel::Animation(index).from_asset(path.clone()));
                (name.clone(), graph.add_clip(clip, 1.0, graph.root))
            })
            .collect();
        let graph = world.resource_mut::<Assets<AnimationGraph>>().add(graph);
        let scene = assets.load(GltfAssetLabel::Scene(0).from_asset(path));
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
                nodes,
                graph,
                player: None,
                playing: None,
                turn: Turn {
                    from: facing,
                    to: facing,
                    started: now,
                },
            },
        ));
    }
}

/// A rig's scene, once spawned, hands its animation player the rig's graph.
fn adopt_player(
    ready: On<WorldInstanceReady>,
    children: Query<&Children>,
    players: Query<(), With<AnimationPlayer>>,
    mut puppets: Query<&mut Puppet>,
    mut commands: Commands,
) {
    let Ok(mut puppet) = puppets.get_mut(ready.entity) else {
        return;
    };
    let Some(player) = children
        .iter_descendants(ready.entity)
        .find(|entity| players.contains(*entity))
    else {
        return;
    };
    commands.entity(player).insert((
        AnimationGraphHandle(puppet.graph.clone()),
        AnimationTransitions::new(),
    ));
    puppet.player = Some(player);
    puppet.playing = None;
}

/// Places every rig where its actor shows, turned the way it faces, playing the clip of what it
/// does, and sounds the cues its clip crosses.
fn pose(world: &mut World) {
    let content = world.resource::<Content>().clone();
    let now = Seconds(world.resource::<Time>().elapsed_secs());
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
            (entity, *shown, *at, actor.clone(), playing.copied())
        })
        .collect();
    let mut sounds = Vec::new();
    for (entity, shown, at, actor, playing) in shown {
        let model = ActorModel::get(&content, actor.model);
        let ground = ground_height(world, at.0);
        let play = Play {
            action: shown.action,
            pose: actor.pose,
            started: playing
                .filter(|playing| playing.action == shown.action)
                .map(|playing| playing.started),
        };
        let (clip, repeat) = match (shown.action, model.pose(actor.pose)) {
            (Action::Idle, Some((clip, _))) => (clip, RepeatAnimation::Forever),
            (Action::Attack | Action::Dead, _) => {
                (model.clip(shown.action), RepeatAnimation::Never)
            }
            (action, _) => (model.clip(action), RepeatAnimation::Forever),
        };
        let rate = if shown.action == Action::Attack {
            actor.attack_rate.at_least(0.01).0
        } else {
            1.0
        };
        let state = {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            play.hash(&mut hasher);
            hasher.finish()
        };
        let playhead = world.resource_mut::<Animator>().advance(entity, state, now);
        {
            let prev = match playhead.previous {
                Previous::Continued(played) => Some(played),
                Previous::Unseen | Previous::Restarted => None,
            };
            let (sfx, stepped) =
                model.cues(shown.action, prev, playhead.elapsed, actor.attack_rate);
            sounds.extend(sfx.into_iter().map(|id| PlaySfx {
                id: *id,
                place: SfxPlace::World(at.0),
            }));
            if stepped
                && let Some(step) = area::of(world, entity).and_then(|area| area.footstep_at(at.0))
            {
                sounds.push(PlaySfx {
                    id: *step,
                    place: SfxPlace::World(at.0),
                });
            }
        }
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
        let node = puppet.nodes.get(&clip.name).copied();
        let restart = puppet.playing != Some(play);
        if restart {
            puppet.playing = Some(play);
        }
        let player = puppet.player;
        let scale = model.scale();
        if let Some(mut transform) = world.get_mut::<Transform>(entity) {
            transform.translation = world_point(at.0, ground);
            transform.rotation = Quat::from_rotation_y(-(facing.0 + std::f32::consts::FRAC_PI_2));
            transform.scale = Vec3::splat(scale);
        }
        let (Some(player), Some(node)) = (player, node) else {
            continue;
        };
        let Ok(mut entity) = world.get_entity_mut(player) else {
            continue;
        };
        let Ok((mut animation, mut transitions)) =
            entity.get_components_mut::<(&mut AnimationPlayer, &mut AnimationTransitions)>()
        else {
            continue;
        };
        if restart {
            transitions
                .play(&mut animation, node, CROSSFADE)
                .set_repeat(repeat)
                .set_speed(rate);
        } else if let Some(active) = animation.animation_mut(node) {
            active.set_speed(rate);
        }
    }
    for sound in sounds {
        world.write_message(sound);
    }
    world.resource_scope(|world, mut animator: Mut<Animator>| {
        animator.retain(|entity| world.get_entity(entity).is_ok());
    });
}
