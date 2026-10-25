use bevy_ecs::lifecycle::HookContext;
use bevy_ecs::prelude::*;
use bevy_ecs::world::DeferredWorld;
use bevy_platform::collections::HashMap;

use super::navmesh::AGENT_RADIUS;
use crate::core::math::Pos;
use crate::core::units::{MeterPos, Meters};
use crate::systems::area::{AreaId, AreaTag};
use crate::systems::stat;

/// How far apart two bodies stand when they just touch.
pub const SPACING: Meters = Meters(2.0 * AGENT_RADIUS.0);

/// The one place an actor means to stand: where its walk ends while it walks, else where it stands.
/// A spot never blocks a walk; it only steers where approaches, wanderers and placements choose to
/// stop, so bodies spread out rather than pile onto one point.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
#[component(on_insert = Spot::count, on_discard = Spot::uncount)]
pub struct Spot {
    pub area: AreaId,
    pub at: Pos<Meters>,
}

/// Every actor's [`Spot`] by area, kept in step with the spots as they come and go.
#[derive(Resource, Default)]
pub struct Spots(HashMap<AreaId, HashMap<Entity, Pos<Meters>>>);

impl Spots {
    /// How many bodies resting on their spots in `area` a body at `at` would overlap, `except` for
    /// one actor's own.
    pub fn crowding(&self, area: AreaId, at: Pos<Meters>, except: Option<Entity>) -> u16 {
        self.0.get(&area).map_or(0, |spots| {
            spots
                .iter()
                .filter(|(entity, spot)| Some(**entity) != except && overlaps(**spot, at))
                .count() as u16
        })
    }
}

/// Whether bodies resting at `a` and `b` would overlap; two just touching do not.
pub fn overlaps(a: Pos<Meters>, b: Pos<Meters>) -> bool {
    a.distance(b) < Meters(SPACING.0 - TOUCH)
}

const TOUCH: f32 = 1e-3;

impl Spot {
    fn count(mut world: DeferredWorld, context: HookContext) {
        let Some(&spot) = world.get::<Spot>(context.entity) else {
            return;
        };
        world
            .resource_mut::<Spots>()
            .0
            .entry(spot.area)
            .or_default()
            .insert(context.entity, spot.at);
    }

    fn uncount(mut world: DeferredWorld, context: HookContext) {
        let Some(&spot) = world.get::<Spot>(context.entity) else {
            return;
        };
        let mut spots = world.resource_mut::<Spots>();
        if let Some(area) = spots.0.get_mut(&spot.area) {
            area.remove(&context.entity);
            if area.is_empty() {
                spots.0.remove(&spot.area);
            }
        }
    }
}

pub fn spot_of(world: &World, entity: Entity) -> Option<Spot> {
    world.get::<Spot>(entity).copied()
}

/// How many actors other than `entity` have spots in its area a body at `at` would overlap.
pub fn crowding(world: &World, entity: Entity, at: Pos<Meters>) -> u16 {
    let Some(area) = world.get::<AreaTag>(entity).map(|tag| tag.area) else {
        return 0;
    };
    world.resource::<Spots>().crowding(area, at, Some(entity))
}

/// Takes the spot at `pos` for the actor; a fallen actor takes none.
pub(super) fn claim(world: &mut World, entity: Entity, pos: Pos<Meters>) {
    if stat::is_dead(world, entity) {
        world.entity_mut(entity).remove::<Spot>();
        return;
    }
    let Some(area) = world.get::<AreaTag>(entity).map(|tag| tag.area) else {
        return;
    };
    let spot = Spot { area, at: pos };
    if spot_of(world, entity) != Some(spot) {
        world.entity_mut(entity).insert(spot);
    }
}
