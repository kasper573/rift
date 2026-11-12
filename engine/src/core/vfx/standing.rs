//! The vfx a thing plays of its own accord, on the client that draws it, for as long as it
//! stands there: no server spawns them, and each client plays its own.

use bevy::prelude::*;

use super::{VfxId, VfxInstance, VfxParams, VfxSite};
use crate::core::time::server_now;
use crate::core::units::Radians;

/// The vfx a thing plays over and over for as long as it is drawn: the flame of a lantern, the
/// glow of a crystal. Whoever draws the thing says which, and stands the thing by its transform
/// and its [`SiteExtent`](super::render::SiteExtent).
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct StandingVfx(pub Vec<VfxId>);

/// The vfx a thing was last found to play, each already playing.
#[derive(Component)]
struct StandingPlayed(Vec<VfxId>);

/// A vfx playing of the client's own accord on the thing it stands on.
#[derive(Component)]
struct StandsOn(Entity);

/// Starts the vfx of every thing that newly stands or plays others than it did, each playing
/// over and over, and ends those of a thing that is gone.
pub(super) fn play(world: &mut World) {
    let Some(now) = server_now(world) else {
        return;
    };
    let fresh: Vec<(Entity, Vec<VfxId>)> = world
        .query::<(Entity, &StandingVfx, Option<&StandingPlayed>)>()
        .iter(world)
        .filter(|(_, standing, played)| played.is_none_or(|played| played.0 != standing.0))
        .map(|(thing, standing, _)| (thing, standing.0.clone()))
        .collect();
    let restarted: Vec<Entity> = fresh.iter().map(|(thing, _)| *thing).collect();
    let playing: Vec<(Entity, Entity)> = world
        .query::<(Entity, &StandsOn)>()
        .iter(world)
        .map(|(instance, on)| (instance, on.0))
        .collect();
    for (instance, thing) in playing {
        if world.get::<StandingVfx>(thing).is_none() || restarted.contains(&thing) {
            world.despawn(instance);
        }
    }
    for (thing, vfx) in fresh {
        for &vfx in &vfx {
            world.spawn((
                VfxInstance {
                    vfx,
                    site: VfxSite::Object(thing),
                    params: VfxParams::default(),
                    heading: Radians::default(),
                    started: now,
                    until: None,
                },
                StandsOn(thing),
            ));
        }
        world.entity_mut(thing).insert(StandingPlayed(vfx));
    }
}
