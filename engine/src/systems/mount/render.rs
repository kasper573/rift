//! Mounts as the client draws them: a body of the mount's model stood under each rider, the rider
//! hung on the joint of their seat making their seat's move, and both drawn as high as the mount
//! travels, over the land for one that flies and on the water for one that floats.
//!
//! A body of every mount's look waits in a stable out of sight, loaded, dressed and drawn, and a
//! rider who mounts is stood on it: a body dressed only then would leave its rider in the air, or
//! on the water, for the frames its files take to load and its first draw takes to prepare.

use std::collections::{HashMap, HashSet};

use bevy::camera::visibility::NoFrustumCulling;
use bevy::math::Affine3A;
use bevy::prelude::*;

use super::{MountDef, Riding, Travels};
use crate::core::content::Content;
use crate::core::interpolate::RenderValueSystems;
use crate::core::render::transition::WorldViewSystems;
use crate::core::render::world::BodiesOnJoints;
use crate::core::render::world::afloat::{Aboard, Afloat};
use crate::core::render::world::body::Body;
use crate::core::render::{CameraRigSystems, GroundHeights, facing_rotation};
use crate::core::units::{Meters, MetersPerSec, PlaybackRate, Radians, Seconds};
use crate::systems::actor::render::{self as bodies, RenderActor, Seated};
use crate::systems::actor::{Action, Actor, ActorModel, Aloft, Appearance, ModelId, Pose};
use crate::systems::area::AreaTag;
use crate::systems::movement::RenderPosition;
use crate::systems::screen::ClientScreen;

pub struct MountRenderPlugin;

impl Plugin for MountRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                carry
                    .after(RenderValueSystems)
                    .before(CameraRigSystems)
                    .run_if(resource_exists::<GroundHeights>),
                stable.before(bodies::dress).in_set(WorldViewSystems),
            )
                .run_if(in_state(ClientScreen::Area)),
        )
        .add_systems(
            PostUpdate,
            seat.in_set(BodiesOnJoints)
                .run_if(in_state(ClientScreen::Area)),
        )
        .add_systems(OnExit(ClientScreen::Area), turn_out);
    }
}

/// Where a body of every mount's look waits for its rider, far under the world: drawn there, out
/// of every view, so whatever its first draw prepares is ready before a rider takes it, which a
/// body kept hidden would only prepare on its first frame in sight.
#[derive(Component)]
struct Stable;

/// How far under the world the [`Stable`] stands, in meters.
const STABLE_DEPTH: f32 = 10_000.0;

/// A body waiting in the [`Stable`] for a rider of a mount drawn as its look: it stands in no
/// area, and touches nothing of the world.
#[derive(Component)]
pub(crate) struct Stalled;

/// The body of the mount a rider rides, which stands where its rider does.
#[derive(Component)]
pub(crate) struct Steed {
    pub(crate) rider: Entity,
}

/// Where a rider's held gear lies while they ride: on `steed`, the body of their mount drawn as
/// `look`, in the hands of it that `hands` names by their place among the model's hands, the first
/// for the rider's first hand and so on.
#[derive(Clone, Debug, PartialEq)]
pub struct GearPlaces {
    pub steed: Entity,
    pub look: ModelId,
    pub hands: Vec<usize>,
}

/// Where the mount `rider` rides carries their gear, if it carries it and its body stands.
pub fn gear_places(world: &mut World, rider: Entity) -> Option<GearPlaces> {
    let content = world.resource::<Content>().clone();
    let def = world.get::<Riding>(rider)?.mount.get(&content);
    if def.gear.is_empty() {
        return None;
    }
    let steed = steeds(world)
        .into_iter()
        .find(|(_, ridden)| *ridden == rider)
        .map(|(steed, _)| steed)?;
    let model = ActorModel::get(&content, def.look);
    let hands = def
        .gear
        .iter()
        .filter_map(|name| model.hands().iter().position(|hand| hand.name == *name))
        .collect();
    Some(GearPlaces {
        steed,
        look: def.look,
        hands,
    })
}

/// Shows every rider as high as their mount travels: one that flies its height over the land or
/// water below, risen and sunk to over the time it eases, one that floats on the water.
fn carry(world: &mut World) {
    let content = world.resource::<Content>().clone();
    let delta = Seconds(world.resource::<Time>().delta_secs());
    let riders: Vec<(Entity, Riding)> = world
        .query::<(Entity, &Riding)>()
        .iter(world)
        .map(|(rider, riding)| (rider, *riding))
        .collect();
    for (rider, riding) in &riders {
        let Some(at) = world.get::<RenderPosition>(*rider).map(|at| at.0) else {
            continue;
        };
        let ground = world.resource::<GroundHeights>();
        let (land, surface) = (ground.height_at(at), ground.surface_at(at));
        let aloft = match riding.mount.get(&content).travels {
            Travels::Riding => None,
            Travels::Floating { .. } => Some(Aloft(surface)),
            Travels::Flying { height, eases } => {
                let from = world.get::<Aloft>(*rider).map_or(land, |aloft| aloft.0);
                let closed = if eases.0 > 0.0 {
                    1.0 - (-3.0 * delta.0 / eases.0).exp()
                } else {
                    1.0
                };
                let risen = from.0 + (surface.0 + height.0 - from.0) * closed;
                Some(Aloft(Meters(risen.max(surface.0))))
            }
        };
        let floats = matches!(riding.mount.get(&content).travels, Travels::Floating { .. });
        let mut rider = world.entity_mut(*rider);
        match aloft {
            Some(aloft) => rider.insert(aloft),
            None => rider.remove::<Aloft>(),
        };
        if floats {
            rider.insert((Afloat { level: surface }, Aboard));
        } else {
            rider.remove::<(Afloat, Aboard)>();
        }
    }
    let landed: Vec<Entity> = world
        .query_filtered::<Entity, (With<Aloft>, With<Actor>, Without<Riding>, Without<Steed>)>()
        .iter(world)
        .collect();
    for landed in landed {
        world.entity_mut(landed).remove::<(Aloft, Afloat, Aboard)>();
    }
}

/// Stands a body of the mount's model under every rider, shown where its rider shows and moving
/// as its rider moves, and has every rider make their seat's move.
fn stable(world: &mut World) {
    let content = world.resource::<Content>().clone();
    let riders: Vec<(Entity, Riding)> = world
        .query::<(Entity, &Riding)>()
        .iter(world)
        .map(|(rider, riding)| (rider, *riding))
        .collect();
    let mut steeds = steeds(world);
    for (steed, rider) in steeds.clone() {
        let looks = world
            .get::<Riding>(rider)
            .map(|riding| riding.mount.get(&content).look);
        let stands = world.get::<Appearance>(steed).map(|shown| shown.model);
        if looks.is_none() || looks != stands {
            world.despawn(steed);
            steeds.retain(|(_, ridden)| *ridden != rider);
        }
    }
    for (rider, riding) in &riders {
        let def = riding.mount.get(&content);
        world
            .entity_mut(*rider)
            .insert(Seated(def.rider.rides.clone()));
        let steed = steeds
            .iter()
            .find(|(_, ridden)| ridden == rider)
            .map(|(steed, _)| *steed)
            .unwrap_or_else(|| {
                let steed = untie(world, def.look)
                    .unwrap_or_else(|| world.spawn(Appearance::of(def.look)).id());
                world.entity_mut(steed).insert(Steed { rider: *rider });
                steed
            });
        lead(world, steed, *rider, def);
    }
    restock(world, &content);
    let stood: Vec<Entity> = world
        .query_filtered::<Entity, (With<Seated>, Without<Riding>)>()
        .iter(world)
        .collect();
    for stood in stood {
        world.entity_mut(stood).remove::<Seated>();
    }
}

/// Shows `steed` where `rider` shows, facing as they face, at the gait its own body covers the
/// ground with as fast as they go.
fn lead(world: &mut World, steed: Entity, rider: Entity, def: &MountDef) {
    let content = world.resource::<Content>().clone();
    let (Some(shown), Some(at), Some(actor)) = (
        world.get::<RenderActor>(rider).copied(),
        world.get::<RenderPosition>(rider).copied(),
        world.get::<Actor>(rider).cloned(),
    ) else {
        return;
    };
    let action = if shown.action.moves() {
        ActorModel::get(&content, def.look).gait(None, actor.speed)
    } else {
        Action::Idle
    };
    let aloft = world.get::<Aloft>(rider).copied();
    let area = world.get::<AreaTag>(rider).cloned();
    let mut steed = world.entity_mut(steed);
    steed.insert((
        RenderActor {
            facing: shown.facing,
            action,
        },
        at,
        Actor {
            facing: shown.facing,
            action,
            pose: Pose::Idle,
            attack_rate: PlaybackRate::default(),
            speed: actor.speed,
        },
    ));
    match aloft {
        Some(aloft) => steed.insert(aloft),
        None => steed.remove::<Aloft>(),
    };
    let afloat = aloft
        .filter(|_| matches!(def.travels, Travels::Floating { .. }))
        .map(|aloft| Afloat { level: aloft.0 });
    match afloat {
        Some(afloat) => steed.insert(afloat),
        None => steed.remove::<Afloat>(),
    };
    if let Some(area) = area {
        steed.insert(area);
    }
}

/// Hangs every rider on the joint of their seat, as their mount stands posed this frame. A rider
/// whose mount has no rig yet sits where the mount stands.
fn seat(world: &mut World) {
    let content = world.resource::<Content>().clone();
    let steeds: HashMap<Entity, Entity> = steeds(world)
        .into_iter()
        .map(|(steed, rider)| (rider, steed))
        .collect();
    let riders: Vec<(Entity, Riding)> = world
        .query::<(Entity, &Riding)>()
        .iter(world)
        .map(|(rider, riding)| (rider, *riding))
        .collect();
    for (rider, riding) in riders {
        let Some(&steed) = steeds.get(&rider) else {
            continue;
        };
        let Some(stands) = world.get::<Transform>(steed).copied() else {
            continue;
        };
        let def = riding.mount.get(&content);
        let joint = world
            .get::<Body>(steed)
            .and_then(|body| body.joint(def.rider.joint.as_str()));
        let on_mount = joint.and_then(|joint| placed_under(world, joint, steed));
        let sits = match on_mount {
            Some(on_mount) => stands.compute_affine() * on_mount,
            None => stands.compute_affine(),
        };
        let (_, seated, translation) = sits.to_scale_rotation_translation();
        // Whoever strikes, casts or works from a seat turns on it to what they aim at.
        let rotation = world
            .get::<RenderActor>(rider)
            .filter(|shown| shown.action == Action::Attack)
            .map_or(seated, |shown| facing_rotation(shown.facing));
        if let Some(mut placed) = world.get_mut::<Transform>(rider) {
            let scale = placed.scale;
            placed.set_if_neq(Transform {
                translation,
                rotation,
                scale,
            });
        }
    }
}

/// Takes a body drawn as `look` out of the stable, one already drawn whole when one is.
fn untie(world: &mut World, look: ModelId) -> Option<Entity> {
    let mut waiting: Vec<(Entity, bool)> = world
        .query_filtered::<(Entity, &Appearance, Option<&Body>), With<Stalled>>()
        .iter(world)
        .filter(|(_, shown, _)| shown.model == look)
        .map(|(stalled, _, body)| (stalled, body.is_some_and(Body::drawn)))
        .collect();
    waiting.sort_by_key(|(_, drawn)| !drawn);
    let (stalled, _) = *waiting.first()?;
    world.entity_mut(stalled).remove::<(Stalled, ChildOf)>();
    for part in descendants(world, stalled) {
        world.entity_mut(part).remove::<NoFrustumCulling>();
    }
    Some(stalled)
}

/// Keeps a body of every mount's look waiting in the stable, and none of a look no mount has.
fn restock(world: &mut World, content: &Content) {
    let looks: HashSet<ModelId> = content
        .table::<MountDef>()
        .rows()
        .map(|mount| mount.look)
        .collect();
    let stable = match world
        .query_filtered::<Entity, With<Stable>>()
        .iter(world)
        .next()
    {
        Some(stable) => stable,
        None => world
            .spawn((
                Stable,
                Transform::from_xyz(0.0, -STABLE_DEPTH, 0.0),
                Visibility::default(),
            ))
            .id(),
    };
    let waiting: Vec<(Entity, ModelId)> = world
        .query_filtered::<(Entity, &Appearance), With<Stalled>>()
        .iter(world)
        .map(|(stalled, shown)| (stalled, shown.model))
        .collect();
    for (stalled, look) in &waiting {
        if !looks.contains(look) {
            world.despawn(*stalled);
        }
    }
    for (stalled, _) in &waiting {
        let meshes: Vec<Entity> = descendants(world, *stalled)
            .into_iter()
            .filter(|part| {
                world.get::<Mesh3d>(*part).is_some()
                    && world.get::<NoFrustumCulling>(*part).is_none()
            })
            .collect();
        for mesh in meshes {
            world.entity_mut(mesh).insert(NoFrustumCulling);
        }
    }
    for look in looks {
        if waiting.iter().any(|(_, waits)| *waits == look) {
            continue;
        }
        world.spawn((
            Stalled,
            Appearance::of(look),
            Actor {
                facing: Radians(0.0),
                action: Action::Idle,
                pose: Pose::Idle,
                attack_rate: PlaybackRate::default(),
                speed: MetersPerSec::default(),
            },
            ChildOf(stable),
        ));
    }
}

fn turn_out(world: &mut World) {
    for (steed, _) in steeds(world) {
        world.despawn(steed);
    }
}

/// Where `node` stands in the space of `root`, an ancestor of it, by the transforms between them
/// as they stand now, ahead of their propagation.
fn placed_under(world: &World, node: Entity, root: Entity) -> Option<Affine3A> {
    let mut placed = Affine3A::IDENTITY;
    let mut at = node;
    while at != root {
        placed = world.get::<Transform>(at)?.compute_affine() * placed;
        at = world.get::<ChildOf>(at)?.parent();
    }
    Some(placed)
}

fn descendants(world: &World, of: Entity) -> Vec<Entity> {
    let mut found = Vec::new();
    let mut left = vec![of];
    while let Some(next) = left.pop() {
        if let Some(children) = world.get::<Children>(next) {
            left.extend(children.iter());
            found.extend(children.iter());
        }
    }
    found
}

fn steeds(world: &mut World) -> Vec<(Entity, Entity)> {
    world
        .query::<(Entity, &Steed)>()
        .iter(world)
        .map(|(steed, of)| (steed, of.rider))
        .collect()
}
