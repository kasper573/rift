//! Mounts as the client draws them: a body of the mount's model stood under each rider, the rider
//! hung on the joint of their seat making their seat's move, and both drawn as high as the mount
//! travels, over the land for one that flies and on the water for one that floats.

use std::collections::HashMap;

use bevy::prelude::*;

use super::{MountDef, Riding, Travels};
use crate::core::content::Content;
use crate::core::interpolate::RenderValueSystems;
use crate::core::render::transition::WorldViewSystems;
use crate::core::render::world::afloat::Afloat;
use crate::core::render::world::body::Body;
use crate::core::render::{CameraRigSystems, GroundHeights, facing_rotation};
use crate::core::units::{Meters, PlaybackRate, Seconds};
use crate::systems::actor::render::{self as bodies, RenderActor, Seated};
use crate::systems::actor::{Action, Actor, ActorModel, Aloft, Appearance, Pose};
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
                (
                    stable.before(bodies::dress),
                    seat.after(bodies::pose).before(bodies::hang),
                )
                    .in_set(WorldViewSystems),
            )
                .run_if(in_state(ClientScreen::Area)),
        )
        .add_systems(OnExit(ClientScreen::Area), turn_out);
    }
}

/// The body of the mount a rider rides, which stands where its rider does.
#[derive(Component)]
struct Steed {
    rider: Entity,
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
            rider.insert(Afloat);
        } else {
            rider.remove::<Afloat>();
        }
    }
    let landed: Vec<Entity> = world
        .query_filtered::<Entity, (With<Aloft>, With<Actor>, Without<Riding>, Without<Steed>)>()
        .iter(world)
        .collect();
    for landed in landed {
        world.entity_mut(landed).remove::<(Aloft, Afloat)>();
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
                let steed = Steed { rider: *rider };
                world.spawn((steed, Appearance::of(def.look))).id()
            });
        lead(world, steed, *rider, def);
    }
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
    match def.travels {
        Travels::Floating { .. } => steed.insert(Afloat),
        Travels::Riding | Travels::Flying { .. } => steed.remove::<Afloat>(),
    };
    if let Some(area) = area {
        steed.insert(area);
    }
}

/// Hangs every rider on the joint of their seat: where the joint stood on its mount as the mount
/// was last drawn, on the mount as it stands now. A rider whose mount has no rig yet sits where
/// the mount stands.
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
        let on_mount = joint
            .and_then(|joint| world.get::<GlobalTransform>(joint))
            .zip(world.get::<GlobalTransform>(steed))
            .map(|(joint, drawn)| drawn.affine().inverse() * joint.affine());
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

fn turn_out(world: &mut World) {
    for (steed, _) in steeds(world) {
        world.despawn(steed);
    }
}

fn steeds(world: &mut World) -> Vec<(Entity, Entity)> {
    world
        .query::<(Entity, &Steed)>()
        .iter(world)
        .map(|(steed, of)| (steed, of.rider))
        .collect()
}
