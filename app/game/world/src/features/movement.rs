use bevy_ecs::message::Messages;
use bevy_ecs::prelude::*;
use bevy_replicon::prelude::FromClient;
use bevy_time::Time;

use crate::core::area::{self, AreaId};
use crate::core::math::{Direction, Pos, Tiles, TilesPerSec};
use crate::core::protocol::{
    ACTION_RUN, ACTION_WALK, AreaTag, MoveRequest, MoveToPortal, Position, is_dead, position,
    set_facing,
};
use crate::features::combat::AttackTarget;
use crate::features::player::sender_player;

#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub pos: Pos<i32>,
}

impl Cell {
    fn center(&self) -> Pos<Tiles> {
        Pos::new(
            Tiles(self.pos.x as f32 + 0.5),
            Tiles(self.pos.y as f32 + 0.5),
        )
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

/// Brings `entity` to rest without stranding it between tiles. Mid-step it keeps only the tile it is
/// entering, so movement lands it on that tile's center; at (or a hair from) a tile it snaps onto the
/// exact center and drops the route. Stopping funnels through here (a redirect re-routes instead), so
/// a resting actor always lands on an exact tile.
pub fn halt(world: &mut World, entity: Entity) {
    world.entity_mut(entity).remove::<MoveTarget>();
    if on_tile(world, entity) {
        // A mid-step stop can land a hair short of the center; snap exactly onto the tile.
        if let Some(at) = position(world, entity) {
            world.entity_mut(entity).insert(Position {
                pos: at.map(|t| t.floor() + 0.5),
            });
        }
        world.entity_mut(entity).remove::<Path>();
    } else if let Some(mut path) = world.get_mut::<Path>(entity) {
        path.tiles.truncate(1);
    }
}

/// Whether `entity` is on a tile center on both axes (within a hair), i.e. not mid-step.
pub fn on_tile(world: &World, entity: Entity) -> bool {
    position(world, entity).is_some_and(|p| centered(p.x.0) && centered(p.y.0))
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
    let dt = world.resource::<Time>().delta_secs();
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

        let speed = world.get::<Speed>(id).map_or(1.0, |s| s.value.0.0);
        let Some(mut at) = position(world, id) else {
            continue;
        };
        // Move the path out (no clone) and reinsert it below, reusing the same allocation.
        let Some(Path { mut tiles }) = world.entity_mut(id).take::<Path>() else {
            continue;
        };
        let mut remaining = speed * dt;
        let mut heading: Option<Pos<Tiles>> = None;
        while remaining > 1e-6 {
            let target = match tiles.first() {
                Some(cell) => cell.center(),
                None => break,
            };
            let step = target - at;
            let distance = step.length();
            if distance < 1e-4 {
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
                at = at + step.normalized().scale(remaining);
                heading = Some(step);
                remaining = 0.0;
            }
        }

        world.entity_mut(id).insert(Position { pos: at });
        if let Some(step) = heading
            && let Some(mut actor) = world.get_mut::<crate::core::protocol::Actor>(id)
        {
            set_facing(
                &mut actor,
                Direction::from_vec(step.x.0, step.y.0) as u8,
                if speed >= 2.0 {
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
        .map_or(AreaId(0), |tag| tag.area);
    let area = &area::areas()[area_id.0 as usize];
    let at = position(world, entity)?;
    let goal = area.grid.nearest_walkable(goal)?;
    let mut path = crate::core::nav::astar(&area.grid, at, goal)?;
    if path.len() > 1 {
        path.remove(0);
    }
    Some(
        path.into_iter()
            .map(|p| Cell {
                pos: Pos::new(p.x.0 as i32, p.y.0 as i32),
            })
            .collect(),
    )
}

fn cross_portal(world: &mut World, entity: Entity) {
    let Some(want) = world.get::<DesiredPortal>(entity).map(|d| d.index as usize) else {
        return;
    };
    let area_id = world
        .get::<AreaTag>(entity)
        .map_or(AreaId(0), |tag| tag.area);
    let Some(portal) = area::areas()[area_id.0 as usize].portals.get(want) else {
        world.entity_mut(entity).remove::<DesiredPortal>();
        return;
    };
    let (dest_area, dest, rect) = (portal.dest_area, portal.dest, portal.rect);
    let Some(at) = position(world, entity) else {
        return;
    };
    if rect.contains(at) {
        if let Some(mut tag) = world.get_mut::<AreaTag>(entity) {
            tag.area = dest_area;
        }
        if let Some(mut p) = world.get_mut::<Position>(entity) {
            p.pos = dest.map(|t| t + 0.5);
        }
        world.entity_mut(entity).remove::<DesiredPortal>();
        forget(world, entity);
    }
}

fn centered(coord: f32) -> bool {
    (coord - coord.floor() - 0.5).abs() < 1e-3
}
