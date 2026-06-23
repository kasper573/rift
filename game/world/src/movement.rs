//! Movement: an entity's replicated [`Position`] and the client's move requests, plus the server
//! systems that path-find, advance movers along tiles, snap them to centers, and cross portals.

use bevy_app::App;
use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_ecs::message::Message;
use bevy_ecs::world::World;
use serde::{Deserialize, Serialize};

use crate::core::math::Pos;
use crate::core::tiling::Tiles;

use crate::actor::{Action, set_facing};
use crate::area::{self, AreaTag};
use crate::combat::{AttackTarget, is_dead};
use crate::core::math::{Direction, Offset};
use crate::core::table::Id;
use crate::core::tiling::{Cell, CellPos, TilePos, TilesPerSec};
use crate::core::time::Seconds;
use crate::player::sender_player;
use bevy_ecs::message::Messages;
use bevy_ecs::prelude::*;
use bevy_replicon::prelude::FromClient;
use bevy_time::Time;

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;

    app.replicate::<Position>()
        .add_client_message::<MoveRequest>(Channel::Ordered)
        .add_client_message::<MoveToPortal>(Channel::Ordered);
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Position {
    pub pos: Pos<Tiles>,
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MoveRequest {
    pub pos: Pos<Tiles>,
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MoveToPortal {
    pub pos: Pos<Tiles>,
    pub portal: u32,
}

pub fn position(world: &World, entity: Entity) -> Option<Pos<Tiles>> {
    world.get::<Position>(entity).map(|p| p.pos)
}

const RUN_SPEED: TilesPerSec = TilesPerSec(Tiles(2.0));

#[derive(Component, Clone, Debug, PartialEq)]
pub struct Path {
    pub tiles: Vec<CellPos>,
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
            world.entity_mut(entity).insert(Position { pos: at.snap() });
        }
        world.entity_mut(entity).remove::<Path>();
    } else if let Some(mut path) = world.get_mut::<Path>(entity) {
        path.tiles.truncate(1);
    }
}

pub fn on_tile(world: &World, entity: Entity) -> bool {
    position(world, entity).is_some_and(|p| p.on_center())
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
            let distance = at.distance(target);
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
                at = at.toward(target, remaining);
                heading = Some(step);
                remaining = Tiles(0.0);
            }
        }

        world.entity_mut(id).insert(Position { pos: at });
        if let Some(step) = heading
            && let Some(mut actor) = world.get_mut::<crate::actor::Actor>(id)
        {
            set_facing(
                &mut actor,
                Direction::from(step),
                if speed >= RUN_SPEED {
                    Action::Run
                } else {
                    Action::Walk
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

fn route(world: &mut World, entity: Entity, goal: Pos<Tiles>) -> Option<Vec<CellPos>> {
    let area_id = world
        .get::<AreaTag>(entity)
        .map_or(Id::new(0), |tag| tag.area);
    let area = &area::areas()[area_id.index()];
    let at = position(world, entity)?;
    let goal = area.grid.nearest_walkable(goal)?;
    let mut path = crate::core::nav::astar(&area.grid, at, goal)?;
    if path.len() > 1 {
        path.remove(0);
    }
    Some(path)
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
            .insert(crate::area::transition::Crossing { dest_area, dest });
        forget(world, entity);
    }
}
