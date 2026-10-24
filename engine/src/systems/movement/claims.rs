use bevy_ecs::lifecycle::HookContext;
use bevy_ecs::prelude::*;
use bevy_ecs::world::DeferredWorld;
use bevy_platform::collections::HashMap;

use crate::core::math::Pos;
use crate::core::units::{CellPos, Meters, TilePos};
use crate::systems::area::{AreaId, AreaTag};
use crate::systems::stat;

/// The one tile an actor means to stand on: where its walk ends while it walks, else where it
/// stands. A claim never blocks a walk; it only steers where approaches, wanderers and placements
/// choose to stop, so actors spread over tiles rather than pile onto one.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
#[component(on_insert = TileClaim::count, on_discard = TileClaim::uncount)]
pub struct TileClaim {
    pub area: AreaId,
    pub cell: CellPos,
}

/// How many actors claim each tile, kept in step with every [`TileClaim`] as it comes and goes.
#[derive(Resource, Default)]
pub struct TileClaims(HashMap<(AreaId, CellPos), u16>);

impl TileClaims {
    pub fn on(&self, area: AreaId, cell: CellPos) -> u16 {
        self.0.get(&(area, cell)).copied().unwrap_or(0)
    }

    pub fn claimed(&self) -> impl Iterator<Item = ((AreaId, CellPos), u16)> + '_ {
        self.0.iter().map(|(&tile, &count)| (tile, count))
    }
}

impl TileClaim {
    pub fn at(area: AreaId, pos: Pos<Meters>) -> TileClaim {
        TileClaim {
            area,
            cell: pos.cell(),
        }
    }

    fn count(mut world: DeferredWorld, context: HookContext) {
        let Some(&claim) = world.get::<TileClaim>(context.entity) else {
            return;
        };
        *world
            .resource_mut::<TileClaims>()
            .0
            .entry((claim.area, claim.cell))
            .or_default() += 1;
    }

    fn uncount(mut world: DeferredWorld, context: HookContext) {
        let Some(&claim) = world.get::<TileClaim>(context.entity) else {
            return;
        };
        let mut claims = world.resource_mut::<TileClaims>();
        let key = (claim.area, claim.cell);
        if let Some(count) = claims.0.get_mut(&key) {
            *count -= 1;
            if *count == 0 {
                claims.0.remove(&key);
            }
        }
    }
}

pub fn claim_of(world: &World, entity: Entity) -> Option<TileClaim> {
    world.get::<TileClaim>(entity).copied()
}

/// How many actors other than `entity` claim `cell` in its area.
pub fn others_on(world: &World, entity: Entity, cell: CellPos) -> u16 {
    let Some(area) = world.get::<AreaTag>(entity).map(|tag| tag.area) else {
        return 0;
    };
    let own = claim_of(world, entity).is_some_and(|claim| claim.area == area && claim.cell == cell);
    world.resource::<TileClaims>().on(area, cell) - u16::from(own)
}

/// Claims `pos`'s tile for the actor; a fallen actor claims nothing.
pub(super) fn claim(world: &mut World, entity: Entity, pos: Pos<Meters>) {
    if stat::is_dead(world, entity) {
        world.entity_mut(entity).remove::<TileClaim>();
        return;
    }
    let Some(area) = world.get::<AreaTag>(entity).map(|tag| tag.area) else {
        return;
    };
    let claim = TileClaim::at(area, pos);
    if claim_of(world, entity) != Some(claim) {
        world.entity_mut(entity).insert(claim);
    }
}
