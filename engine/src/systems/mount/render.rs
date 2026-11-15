//! Mounts as the client draws them: a body of the mount's model stood under each driver, every
//! rider hung on the joint of their seat making their seat's move, and all of them drawn as high
//! as the mount travels, over the land for one that flies and on the water for one that floats.

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

/// The body of the mount a driver rides, which stands where its driver does.
#[derive(Component)]
struct Steed {
    driver: Entity,
}

/// Shows every passenger where their driver shows, and every rider as high as their mount
/// travels: one that flies its height over the land or water below, risen and sunk to over the
/// time it eases, one that floats on the water.
fn carry(world: &mut World) {
    let content = world.resource::<Content>().clone();
    let delta = Seconds(world.resource::<Time>().delta_secs());
    let riders: Vec<(Entity, Riding)> = world
        .query::<(Entity, &Riding)>()
        .iter(world)
        .map(|(rider, riding)| (rider, *riding))
        .collect();
    let mut heights: HashMap<Entity, Option<Aloft>> = HashMap::new();
    for (driver, riding) in riders.iter().filter(|(_, riding)| riding.drives()) {
        let Some(at) = world.get::<RenderPosition>(*driver).map(|at| at.0) else {
            continue;
        };
        let ground = world.resource::<GroundHeights>();
        let (land, surface) = (ground.height_at(at), ground.surface_at(at));
        let aloft = match riding.mount.get(&content).travels {
            Travels::Riding => None,
            Travels::Floating { .. } => Some(Aloft(surface)),
            Travels::Flying { height, eases } => {
                let from = world.get::<Aloft>(*driver).map_or(land, |aloft| aloft.0);
                let closed = if eases.0 > 0.0 {
                    1.0 - (-3.0 * delta.0 / eases.0).exp()
                } else {
                    1.0
                };
                let risen = from.0 + (surface.0 + height.0 - from.0) * closed;
                Some(Aloft(Meters(risen.max(surface.0))))
            }
        };
        heights.insert(*driver, aloft);
    }
    for (rider, riding) in &riders {
        let shown = world.get::<RenderPosition>(riding.driver).copied();
        let aloft = heights.get(&riding.driver).copied().flatten();
        let floats = matches!(riding.mount.get(&content).travels, Travels::Floating { .. });
        let mut rider = world.entity_mut(*rider);
        if let (false, Some(shown)) = (riding.drives(), shown) {
            rider.insert(shown);
        }
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

/// Stands a body of the mount's model under every driver, shown where its driver shows and
/// moving as its driver moves, and has every rider make their seat's move.
fn stable(world: &mut World) {
    let content = world.resource::<Content>().clone();
    let riders: Vec<(Entity, Riding)> = world
        .query::<(Entity, &Riding)>()
        .iter(world)
        .map(|(rider, riding)| (rider, *riding))
        .collect();
    let mut steeds = steeds(world);
    for (steed, driver) in steeds.clone() {
        let looks = world
            .get::<Riding>(driver)
            .filter(|riding| riding.drives())
            .map(|riding| riding.mount.get(&content).look);
        let stands = world.get::<Appearance>(steed).map(|shown| shown.model);
        if looks.is_none() || looks != stands {
            world.despawn(steed);
            steeds.retain(|(_, driven)| *driven != driver);
        }
    }
    for (rider, riding) in &riders {
        let def = riding.mount.get(&content);
        match def.seats.get(usize::from(riding.seat.0)) {
            Some(seat) => world.entity_mut(*rider).insert(Seated(seat.rides.clone())),
            None => world.entity_mut(*rider).remove::<Seated>(),
        };
        if !riding.drives() {
            continue;
        }
        let steed = steeds
            .iter()
            .find(|(_, driver)| driver == rider)
            .map(|(steed, _)| *steed)
            .unwrap_or_else(|| {
                let steed = Steed { driver: *rider };
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

/// Shows `steed` where `driver` shows, facing as they face, at the gait its own body covers the
/// ground with as fast as they go.
fn lead(world: &mut World, steed: Entity, driver: Entity, def: &MountDef) {
    let content = world.resource::<Content>().clone();
    let (Some(shown), Some(at), Some(actor)) = (
        world.get::<RenderActor>(driver).copied(),
        world.get::<RenderPosition>(driver).copied(),
        world.get::<Actor>(driver).cloned(),
    ) else {
        return;
    };
    let action = if shown.action.moves() {
        ActorModel::get(&content, def.look).gait(None, actor.speed)
    } else {
        Action::Idle
    };
    let aloft = world.get::<Aloft>(driver).copied();
    let area = world.get::<AreaTag>(driver).cloned();
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
        .map(|(steed, driver)| (driver, steed))
        .collect();
    let riders: Vec<(Entity, Riding)> = world
        .query::<(Entity, &Riding)>()
        .iter(world)
        .map(|(rider, riding)| (rider, *riding))
        .collect();
    for (rider, riding) in riders {
        let Some(&steed) = steeds.get(&riding.driver) else {
            continue;
        };
        let Some(stands) = world.get::<Transform>(steed).copied() else {
            continue;
        };
        let def = riding.mount.get(&content);
        let joint = def
            .seats
            .get(usize::from(riding.seat.0))
            .zip(world.get::<Body>(steed))
            .and_then(|(seat, body)| body.joint(seat.joint.as_str()));
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
        .map(|(steed, of)| (steed, of.driver))
        .collect()
}
