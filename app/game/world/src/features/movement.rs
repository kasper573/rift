use std::collections::HashSet;

use rift::{Builder, ClientId, Ctx, Entity, Wire, World};

use crate::core::area::{self, AreaId};
use crate::core::math::{Direction, Pos, Tiles, TilesPerSec};
use crate::core::protocol::{
    ACTION_RUN, ACTION_WALK, AreaTag, Position, is_dead, position, set_facing,
};
use crate::features::combat::AttackTarget;
use crate::features::player::Players;

#[derive(Wire, Clone, Debug, PartialEq)]
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

#[derive(Wire, Clone, Debug, PartialEq)]
pub struct Path {
    pub tiles: Vec<Cell>,
}

#[derive(Wire, Clone, Debug, PartialEq)]
pub struct MoveTarget {
    pub pos: Pos<Tiles>,
}

#[derive(Wire, Clone, Debug, PartialEq)]
pub struct Speed {
    pub value: TilesPerSec,
}

#[derive(Wire, Clone, Debug, PartialEq)]
pub struct DesiredPortal {
    pub index: u32,
}

#[derive(Wire, Clone, Debug, PartialEq)]
pub struct MoveRequest {
    pub pos: Pos<Tiles>,
}

#[derive(Wire, Clone, Debug, PartialEq)]
pub struct MoveToPortal {
    pub pos: Pos<Tiles>,
    pub portal: u32,
}

pub fn input(b: &mut Builder) {
    b.intent(move_request);
    b.intent(move_to_portal);
}

pub fn step(b: &mut Builder) {
    b.system(advance);
}

pub fn forget(world: &mut World, entity: Entity) {
    world.remove::<AttackTarget>(entity);
    world.remove::<DesiredPortal>(entity);
    halt(world, entity);
}

/// Brings `entity` to rest without stranding it between tiles. Mid-step it keeps only the tile it is
/// entering, so movement lands it on that tile's center; at (or a hair from) a tile it snaps onto the
/// exact center and drops the route. Stopping funnels through here (a redirect re-routes instead), so
/// a resting actor always lands on an exact tile.
pub fn halt(world: &mut World, entity: Entity) {
    world.remove::<MoveTarget>(entity);
    if on_tile(world, entity) {
        // A mid-step stop can land a hair short of the center; snap exactly onto the tile.
        if let Some(at) = position(world, entity) {
            world.insert(
                entity,
                Position {
                    pos: at.map(|t| t.floor() + 0.5),
                },
            );
        }
        world.remove::<Path>(entity);
    } else if let Some(mut path) = world.get::<Path>(entity) {
        path.tiles.truncate(1);
        world.insert(entity, path);
    }
}

/// Whether `entity` is on a tile center on both axes (within a hair), i.e. not mid-step.
pub fn on_tile(world: &World, entity: Entity) -> bool {
    position(world, entity).is_some_and(|p| centered(p.x.0) && centered(p.y.0))
}

fn move_request(ctx: &mut Ctx) {
    for (client, req) in ctx.server.drain_events::<MoveRequest>() {
        if let Some(entity) = retarget(ctx, client, req.pos) {
            ctx.server.world.remove::<DesiredPortal>(entity);
        }
    }
}

fn move_to_portal(ctx: &mut Ctx) {
    for (client, req) in ctx.server.drain_events::<MoveToPortal>() {
        if let Some(entity) = retarget(ctx, client, req.pos) {
            ctx.server
                .world
                .insert(entity, DesiredPortal { index: req.portal });
        }
    }
}

fn retarget(ctx: &mut Ctx, client: ClientId, pos: Pos<Tiles>) -> Option<Entity> {
    let entity = ctx.res.get::<Players>()?.0.get(&client).copied()?;
    let world = &mut ctx.server.world;
    if is_dead(world, entity) {
        return None;
    }
    world.remove::<AttackTarget>(entity);
    world.remove::<Path>(entity);
    world.insert(entity, MoveTarget { pos });
    Some(entity)
}

fn advance(ctx: &mut Ctx) {
    let dt = ctx.dt;
    let world = &mut ctx.server.world;
    let movers: HashSet<Entity> = world
        .ids::<MoveTarget>()
        .into_iter()
        .chain(world.ids::<Path>())
        .collect();
    for id in movers {
        if !world.alive(id) || is_dead(world, id) {
            world.remove::<MoveTarget>(id);
            world.remove::<Path>(id);
            continue;
        }
        if !world.has::<Path>(id) {
            let Some(goal) = world.get::<MoveTarget>(id).map(|m| m.pos) else {
                continue;
            };
            match route(world, id, goal) {
                Some(tiles) => world.insert(id, Path { tiles }),
                None => {
                    world.remove::<MoveTarget>(id);
                    continue;
                }
            }
        }

        let speed = world.get::<Speed>(id).map_or(1.0, |s| s.value.0.0);
        let Some(mut at) = position(world, id) else {
            continue;
        };
        // Move the path out (no clone) and reinsert it below, reusing the same allocation.
        let Some(Path { mut tiles }) = world.take::<Path>(id) else {
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

        world.insert(id, Position { pos: at });
        if let Some(step) = heading {
            set_facing(
                world,
                id,
                Direction::from_vec(step.x.0, step.y.0) as u8,
                if speed >= 2.0 {
                    ACTION_RUN
                } else {
                    ACTION_WALK
                },
            );
        }
        if tiles.is_empty() {
            world.remove::<MoveTarget>(id);
        } else {
            world.insert(id, Path { tiles });
        }
        cross_portal(world, id);
    }
}

fn route(world: &World, entity: Entity, goal: Pos<Tiles>) -> Option<Vec<Cell>> {
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

// Retargeting into the destination area is the whole crossing: rift sees the changed zone and
// migrates the entity (and its client) to that shard.
fn cross_portal(world: &mut World, entity: Entity) {
    let Some(want) = world.get::<DesiredPortal>(entity).map(|d| d.index as usize) else {
        return;
    };
    let area_id = world
        .get::<AreaTag>(entity)
        .map_or(AreaId(0), |tag| tag.area);
    let Some(portal) = area::areas()[area_id.0 as usize].portals.get(want) else {
        world.remove::<DesiredPortal>(entity);
        return;
    };
    let (dest_area, dest, rect) = (portal.dest_area, portal.dest, portal.rect);
    let Some(at) = position(world, entity) else {
        return;
    };
    if rect.contains(at) {
        world.modify::<AreaTag>(entity, |tag| tag.area = dest_area);
        world.modify::<Position>(entity, |p| p.pos = dest.map(|t| t + 0.5));
        world.remove::<DesiredPortal>(entity);
        forget(world, entity);
    }
}

fn centered(coord: f32) -> bool {
    (coord - coord.floor() - 0.5).abs() < 1e-3
}
