//! How much of a thing shows: a thing that fades thins away with all it holds, rather than
//! vanishing at once.

use bevy::prelude::*;

/// How much of an entity and all that hangs under it shows, from 1 whole to 0 nothing: its meshes
/// thin out pixel by pixel, and its lamps dim.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Opacity(pub f32);

impl Opacity {
    /// How much of `entity` shows: as much as it says, or the nearest thing that holds it does,
    /// and all of it where none says.
    pub fn of(world: &World, entity: Entity) -> f32 {
        let mut at = entity;
        loop {
            if let Some(opacity) = world.get::<Opacity>(at) {
                return opacity.0;
            }
            match world.get::<ChildOf>(at) {
                Some(held) => at = held.parent(),
                None => return 1.0,
            }
        }
    }

    /// How much of `entity` shows, asked of a system's queries.
    pub fn inherited(
        entity: Entity,
        opacities: &Query<&Opacity>,
        holders: &Query<&ChildOf>,
    ) -> f32 {
        std::iter::once(entity)
            .chain(holders.iter_ancestors(entity))
            .find_map(|at| opacities.get(at).ok())
            .map_or(1.0, |opacity| opacity.0)
    }
}
