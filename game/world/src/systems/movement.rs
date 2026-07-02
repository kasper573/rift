use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_ecs::query::QueryState;
use bevy_time::Time;
use serde::{Deserialize, Serialize};

use crate::core::assets::AssetService;
use crate::core::math::{Direction, Offset, Pos};
use crate::core::tiling::{Cell, CellPos, TilePos, Tiles, TilesPerSec};
use crate::core::time::Seconds;
use bevy_terminal::{CommandCtx, command};

use crate::systems::account::identity::Identity;
use crate::systems::account::role;
use crate::systems::actor::{Action, Actor, set_facing};
use crate::systems::area;
use crate::systems::combat::AttackTarget;
use crate::systems::player::{ClientId, Players, conn_player, sender_player};
use crate::systems::stat::{self, StatKind};

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

/// Remaining waypoints, stored back-to-front: the next cell to walk is `tiles.last()`, so advancing
/// pops it off the end in O(1) instead of shifting the whole vec from the front.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct Path {
    pub tiles: Vec<CellPos>,
}

#[derive(Component, Clone, Debug, PartialEq)]
pub struct MoveTarget {
    pub pos: Pos<Tiles>,
}

#[derive(Component, Clone, Debug, PartialEq)]
pub struct DesiredPortal {
    pub index: u32,
}

pub fn move_request(world: &mut World) {
    for request in crate::systems::requests::<MoveRequest>(world) {
        if let Some(entity) = retarget(world, request.client_id, request.message.pos) {
            world.entity_mut(entity).remove::<DesiredPortal>();
        }
    }
}

pub fn move_to_portal(world: &mut World) {
    for request in crate::systems::requests::<MoveToPortal>(world) {
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

pub fn halt(world: &mut World, entity: Entity) {
    world.entity_mut(entity).remove::<MoveTarget>();
    if on_tile(world, entity) {
        if let Some(at) = position(world, entity) {
            world.entity_mut(entity).insert(Position { pos: at.snap() });
        }
        world.entity_mut(entity).remove::<Path>();
    } else if let Some(mut path) = world.get_mut::<Path>(entity) {
        let next = path.tiles.last().copied();
        path.tiles.clear();
        path.tiles.extend(next);
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
    if stat::is_dead(world, entity) {
        return None;
    }
    goto(world, entity, pos);
    Some(entity)
}

pub fn goto(world: &mut World, entity: Entity, pos: Pos<Tiles>) {
    world
        .entity_mut(entity)
        .remove::<AttackTarget>()
        .remove::<Path>()
        .insert(MoveTarget { pos });
}

pub fn approach(world: &mut World, entity: Entity, target: Pos<Tiles>, range: Tiles) -> bool {
    let Some(at) = position(world, entity) else {
        return false;
    };
    if at.distance(target) <= range {
        return true;
    }
    let heading = world
        .get::<MoveTarget>(entity)
        .is_some_and(|goal| goal.pos.distance(target) <= range);
    if !heading {
        let dest = approach_tile(world, entity, at, target, range).unwrap_or(target);
        world
            .entity_mut(entity)
            .remove::<Path>()
            .insert(MoveTarget { pos: dest });
    }
    false
}

fn approach_tile(
    world: &World,
    entity: Entity,
    from: Pos<Tiles>,
    target: Pos<Tiles>,
    range: Tiles,
) -> Option<Pos<Tiles>> {
    let grid = &area::of(world, entity)?.grid;
    let assets = world.resource::<AssetService>();
    let airborne = world.get::<Actor>(entity).is_some_and(|actor| {
        assets
            .resolve(*actor.model.get(), crate::systems::actor::build_model)
            .airborne
    });
    let goal = target.cell();
    let reach = range.0.ceil() as i32;
    let mut best: Option<(Pos<Tiles>, Tiles)> = None;
    for dy in -reach..=reach {
        for dx in -reach..=reach {
            let cell = goal.step((dx, dy));
            if cell == goal {
                continue;
            }
            let center = cell.center();
            if center.distance(target) > range || (!airborne && !grid.walkable(center)) {
                continue;
            }
            let distance = from.distance(center);
            if best.is_none_or(|(_, best)| distance < best) {
                best = Some((center, distance));
            }
        }
    }
    best.map(|(center, _)| center)
}

type Movers = QueryState<Entity, Or<(With<MoveTarget>, With<Path>)>>;
type StepQuery = QueryState<(
    &'static mut Position,
    &'static mut Path,
    Option<&'static mut Actor>,
)>;

pub fn advance(world: &mut World, movers_query: &mut Movers, step_query: &mut StepQuery) {
    let dt = Seconds(world.resource::<Time>().delta_secs());
    let movers: Vec<Entity> = movers_query.iter(world).collect();
    for id in movers {
        if world.get_entity(id).is_err() || stat::is_dead(world, id) {
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

        let speed = TilesPerSec(Tiles(stat::effective(world, id, StatKind::MovementSpeed)));
        // Position, Path and the optional Actor are fetched in one access instead of separate
        // per-component lookups. Identical behavior: Position and Path are required (Path was just
        // ensured above); an actor-less mover still moves, it just has no facing to update.
        let Ok((mut position, mut path, mut actor)) = step_query.get_mut(world, id) else {
            continue;
        };
        let mut at = position.pos;
        let mut heading: Option<Offset<Tiles>> = None;
        let tiles = &mut path.tiles;
        let mut remaining = speed * dt;
        while remaining > Tiles(1e-6) {
            let target = match tiles.last() {
                Some(cell) => cell.center(),
                None => break,
            };
            let step = target - at;
            let distance = at.distance(target);
            if distance < Tiles(1e-4) {
                at = target;
                tiles.pop();
                continue;
            }
            if distance <= remaining {
                at = target;
                remaining -= distance;
                heading = Some(step);
                tiles.pop();
            } else {
                at = at.toward(target, remaining);
                heading = Some(step);
                remaining = Tiles(0.0);
            }
        }
        let arrived = tiles.is_empty();

        position.pos = at;
        if let Some(step) = heading
            && let Some(actor) = actor.as_mut()
        {
            set_facing(
                actor,
                Direction::from(step),
                if speed >= RUN_SPEED {
                    Action::Run
                } else {
                    Action::Walk
                },
            );
        }
        if arrived {
            world.entity_mut(id).remove::<(MoveTarget, Path)>();
        }
        cross_portal(world, id);
    }
}

fn route(world: &mut World, entity: Entity, goal: Pos<Tiles>) -> Option<Vec<CellPos>> {
    let assets = world.resource::<AssetService>();
    if world.get::<Actor>(entity).is_some_and(|actor| {
        assets
            .resolve(*actor.model.get(), crate::systems::actor::build_model)
            .airborne
    }) {
        return Some(vec![goal.cell()]);
    }
    let area = area::of(world, entity)?;
    let at = position(world, entity)?;
    let mut path = crate::core::nav::astar(&area.grid, at, goal)?;
    if path.len() > 1 {
        path.remove(0);
    }
    path.reverse();
    Some(path)
}

fn cross_portal(world: &mut World, entity: Entity) {
    let Some(want) = world.get::<DesiredPortal>(entity).map(|d| d.index as usize) else {
        return;
    };
    let Some(area_id) = world.get::<area::AreaTag>(entity).map(|tag| tag.area) else {
        return;
    };
    let area = world
        .resource::<AssetService>()
        .resolve(area_id.get().map, area::build_area);
    let Some(portal) = area.portals.get(want) else {
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
    relocate(world, entity, dest_area, dest);
}

fn relocate(world: &mut World, entity: Entity, dest_area: area::Id, dest: Pos<Tiles>) {
    let current = world.get::<area::AreaTag>(entity).map(|tag| tag.area);
    if current == Some(dest_area) {
        if let Some(mut position) = world.get_mut::<Position>(entity) {
            position.pos = dest;
        }
    } else {
        world
            .entity_mut(entity)
            .insert(crate::systems::area::transition::Crossing { dest_area, dest });
    }
    forget(world, entity);
}

/// Teleport a player.
#[command(name = "tp", access = role::is_admin)]
fn teleport(
    world: &mut World,
    ctx: &CommandCtx,
    x: f32,
    y: f32,
    area: Option<area::Id>,
    user: Option<String>,
) -> Result<String, String> {
    let target = match &user {
        Some(user) => player_by_user(world, user)
            .ok_or_else(|| format!("no player with user id `{user}` in your area"))?,
        None => conn_player(world, ctx.conn)
            .ok_or_else(|| "you have no player to teleport".to_owned())?,
    };
    let current = world
        .get::<area::AreaTag>(target)
        .map(|tag| tag.area)
        .ok_or_else(|| "target has no area".to_owned())?;
    let dest_area = area.unwrap_or(current);
    let size = world
        .resource::<AssetService>()
        .resolve(dest_area.get().map, area::build_area)
        .size;
    if !(0.0..=size.width).contains(&x) || !(0.0..=size.height).contains(&y) {
        return Err(format!(
            "({x},{y}) is outside {dest_area:?} ({}x{} tiles)",
            size.width, size.height
        ));
    }
    relocate(world, target, dest_area, Pos::new(x, y));
    Ok(format!("teleported to {x},{y} in {dest_area:?}"))
}

fn player_by_user(world: &mut World, user: &str) -> Option<Entity> {
    let client: ClientId = world
        .query::<(&ClientId, &Identity)>()
        .iter(world)
        .find(|(_, identity)| identity.id == user)
        .map(|(&client, _)| client)?;
    world.resource::<Players>().0.get(&client).copied()
}
