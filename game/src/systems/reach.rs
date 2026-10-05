use bevy_ecs::prelude::*;

use crate::core::math::Pos;
use crate::core::tiling::{TilePos, Tiles};
use crate::systems::area::{self, AreaTag};
use crate::systems::movement::{self, approach, position};
use crate::systems::stat;

const DIAGONAL_MARGIN: Tiles = Tiles(std::f32::consts::SQRT_2 - 1.0);

#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct ReachIntent {
    pub target: Entity,
    pub act: ReachAct,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReachAct {
    Attack,
    Pickup,
    Talk,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pursuit {
    Arrived,
    Approaching,
    Lost,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tether {
    pub anchor: Pos<Tiles>,
    pub area: area::Id,
    pub range: Tiles,
    pub npc: Option<Entity>,
}

impl Tether {
    pub fn around(
        world: &World,
        actor: Entity,
        range: Tiles,
        npc: Option<Entity>,
    ) -> Option<Tether> {
        Some(Tether {
            anchor: position(world, actor)?,
            area: world.get::<AreaTag>(actor)?.area,
            range,
            npc,
        })
    }

    pub fn holds(&self, world: &World, actor: Entity) -> bool {
        !stat::is_dead(world, actor)
            && world.get::<AreaTag>(actor).map(|tag| tag.area) == Some(self.area)
            && position(world, actor)
                .is_some_and(|at| at.distance(self.anchor) <= self.range + DIAGONAL_MARGIN)
            && self
                .npc
                .is_none_or(|npc| world.get_entity(npc).is_ok() && !stat::is_dead(world, npc))
    }
}

pub fn intend(world: &mut World, actor: Entity, target: Entity, act: ReachAct) {
    movement::forget(world, actor);
    world.entity_mut(actor).insert(ReachIntent { target, act });
}

pub fn intent(world: &World, actor: Entity, act: ReachAct) -> Option<Entity> {
    world
        .get::<ReachIntent>(actor)
        .filter(|intent| intent.act == act)
        .map(|intent| intent.target)
}

pub fn pursue(world: &mut World, actor: Entity, range: Tiles) -> Pursuit {
    let Some(target) = world.get::<ReachIntent>(actor).map(|intent| intent.target) else {
        return Pursuit::Lost;
    };
    if stat::is_dead(world, actor)
        || world.get_entity(target).is_err()
        || stat::is_dead(world, target)
        || !same_area(world, actor, target)
    {
        return Pursuit::Lost;
    }
    let Some(target_at) = position(world, target) else {
        return Pursuit::Lost;
    };
    if approach(world, actor, target_at, range + DIAGONAL_MARGIN) {
        Pursuit::Arrived
    } else {
        Pursuit::Approaching
    }
}

pub fn forget(world: &mut World, actor: Entity) {
    world.entity_mut(actor).remove::<ReachIntent>();
    movement::forget(world, actor);
}

fn same_area(world: &World, actor: Entity, target: Entity) -> bool {
    let area = |entity| world.get::<AreaTag>(entity).map(|tag| tag.area);
    area(actor).is_some() && area(actor) == area(target)
}
