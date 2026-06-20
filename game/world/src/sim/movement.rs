use bevy_ecs::message::Messages;
use bevy_ecs::prelude::*;
use bevy_replicon::prelude::FromClient;
use bevy_time::Time;

use super::combat::AttackTarget;
use super::player::sender_player;
use crate::area;
use crate::math::{self, CellPos, Direction, Offset, Pos, Seconds, Tiles, TilesPerSec};
use crate::protocol::{
    ACTION_RUN, ACTION_WALK, AreaTag, MoveRequest, MoveToPortal, Position, is_dead, position,
    set_facing,
};
use crate::table::Id;

const RUN_SPEED: TilesPerSec = TilesPerSec(Tiles(2.0));

#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub pos: CellPos,
}

impl Cell {
    fn center(&self) -> Pos<Tiles> {
        math::tile_center(self.pos)
    }
}

#[derive(Component, Clone, Debug, PartialEq)]
pub struct Path {
    pub tiles: Vec<Cell>,
}

#[derive(Component, Clone, Debug, PartialEq)]
pub struct MoveTarget {
    pub pos: Pos<Tiles>,
}

#[derive(Component, Clone, Debug, PartialEq)]
pub struct Speed {
    pub value: TilesPerSec,
}

#[derive(Component, Clone, Debug, PartialEq)]
pub struct DesiredPortal {
    pub index: u32,
}

pub fn move_request(world: &mut World) {
    let requests: Vec<FromClient<MoveRequest>> = world
        .resource_mut::<Messages<FromClient<MoveRequest>>>()
        .drain()
        .collect();
    for request in requests {
        if let Some(entity) = retarget(world, request.client_id, request.message.pos) {
            world.entity_mut(entity).remove::<DesiredPortal>();
        }
    }
}

pub fn move_to_portal(world: &mut World) {
    let requests: Vec<FromClient<MoveToPortal>> = world
        .resource_mut::<Messages<FromClient<MoveToPortal>>>()
        .drain()
        .collect();
    for request in requests {
        if let Some(entity) = retarget(world, request.client_id, request.message.pos) {
            world.entity_mut(entity).insert(DesiredPortal {
                index: request.message.portal,
            });
        }
    }
}

pub fn forget(world: &mut World, entity: Entity) {
    world
        .entity_mut(entity)
        .remove::<AttackTarget>()
        .remove::<DesiredPortal>();
    halt(world, entity);
}

/// Mid-step it keeps only the entering tile so it lands on that tile's center; else snaps to exact center.
/// A resting actor always lands on an exact tile via this funnel.
pub fn halt(world: &mut World, entity: Entity) {
    world.entity_mut(entity).remove::<MoveTarget>();
    if on_tile(world, entity) {
        if let Some(at) = position(world, entity) {
            world.entity_mut(entity).insert(Position {
                pos: math::snap_to_tile(at),
            });
        }
        world.entity_mut(entity).remove::<Path>();
    } else if let Some(mut path) = world.get_mut::<Path>(entity) {
        path.tiles.truncate(1);
    }
}

pub fn on_tile(world: &World, entity: Entity) -> bool {
    position(world, entity).is_some_and(math::on_center)
}

fn retarget(
    world: &mut World,
    sender: bevy_replicon::prelude::ClientId,
    pos: Pos<Tiles>,
) -> Option<Entity> {
    let entity = sender_player(world, sender)?;
    if is_dead(world, entity) {
        return None;
    }
    world
        .entity_mut(entity)
        .remove::<AttackTarget>()
        .remove::<Path>()
        .insert(MoveTarget { pos });
    Some(entity)
}

pub fn advance(world: &mut World) {
    let dt = Seconds(world.resource::<Time>().delta_secs());
    let movers: Vec<Entity> = world
        .query_filtered::<Entity, Or<(With<MoveTarget>, With<Path>)>>()
        .iter(world)
        .collect();
    for id in movers {
        if world.get_entity(id).is_err() || is_dead(world, id) {
            if world.get_entity(id).is_ok() {
                world.entity_mut(id).remove::<(MoveTarget, Path)>();
            }
            continue;
        }
        if world.get::<Path>(id).is_none() {
            let Some(goal) = world.get::<MoveTarget>(id).map(|m| m.pos) else {
                continue;
            };
            match route(world, id, goal) {
                Some(tiles) => {
                    world.entity_mut(id).insert(Path { tiles });
                }
                None => {
                    world.entity_mut(id).remove::<MoveTarget>();
                    continue;
                }
            }
        }

        let speed = world
            .get::<Speed>(id)
            .map_or(TilesPerSec(Tiles(1.0)), |s| s.value);
        let Some(mut at) = position(world, id) else {
            continue;
        };
        let Some(Path { mut tiles }) = world.entity_mut(id).take::<Path>() else {
            continue;
        };
        let mut remaining = speed * dt;
        let mut heading: Option<Offset<Tiles>> = None;
        while remaining > Tiles(1e-6) {
            let target = match tiles.first() {
                Some(cell) => cell.center(),
                None => break,
            };
            let step = target - at;
            let distance = Tiles(step.length());
            if distance < Tiles(1e-4) {
                at = target;
                tiles.remove(0);
                continue;
            }
            if distance <= remaining {
                at = target;
                remaining -= distance;
                heading = Some(step);
                tiles.remove(0);
            } else {
                at += step.normalize() * remaining.0;
                heading = Some(step);
                remaining = Tiles(0.0);
            }
        }

        world.entity_mut(id).insert(Position { pos: at });
        if let Some(step) = heading
            && let Some(mut actor) = world.get_mut::<crate::protocol::Actor>(id)
        {
            set_facing(
                &mut actor,
                Direction::from_vec(step.x, step.y) as u8,
                if speed >= RUN_SPEED {
                    ACTION_RUN
                } else {
                    ACTION_WALK
                },
            );
        }
        if tiles.is_empty() {
            world.entity_mut(id).remove::<MoveTarget>();
        } else {
            world.entity_mut(id).insert(Path { tiles });
        }
        cross_portal(world, id);
    }
}

fn route(world: &mut World, entity: Entity, goal: Pos<Tiles>) -> Option<Vec<Cell>> {
    let area_id = world
        .get::<AreaTag>(entity)
        .map_or(Id::new(0), |tag| tag.area);
    let area = &area::areas()[area_id.index()];
    let at = position(world, entity)?;
    let goal = area.grid.nearest_walkable(goal)?;
    let mut path = crate::nav::astar(&area.grid, at, goal)?;
    if path.len() > 1 {
        path.remove(0);
    }
    Some(path.into_iter().map(|pos| Cell { pos }).collect())
}

fn cross_portal(world: &mut World, entity: Entity) {
    let Some(want) = world.get::<DesiredPortal>(entity).map(|d| d.index as usize) else {
        return;
    };
    let area_id = world
        .get::<AreaTag>(entity)
        .map_or(Id::new(0), |tag| tag.area);
    let Some(portal) = area::areas()[area_id.index()].portals.get(want) else {
        world.entity_mut(entity).remove::<DesiredPortal>();
        return;
    };
    let (dest_area, dest, rect) = (portal.dest_area, portal.dest, portal.rect);
    let Some(at) = position(world, entity) else {
        return;
    };
    if !rect.contains(at) {
        return;
    }
    if dest_area == area_id {
        if let Some(mut p) = world.get_mut::<Position>(entity) {
            p.pos = dest;
        }
        forget(world, entity);
    } else {
        world
            .entity_mut(entity)
            .insert(super::transition::Crossing { dest_area, dest });
        forget(world, entity);
    }
}
