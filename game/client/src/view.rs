use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use world::area;
use world::math::{Pos, Rect, Size, Tiles};
use world::protocol::{Actor, AreaTag, Hitbox, Position, Vitals};
use world::session;

use crate::render::{TILE, Viewport, WorldCamera};

pub fn cursor_tile(world: &mut World) -> Option<Pos<Tiles>> {
    let cursor = world
        .query_filtered::<&Window, With<PrimaryWindow>>()
        .single(world)
        .ok()?
        .cursor_position()?;
    let viewport = *world.resource::<Viewport>();
    if viewport.scale <= 0.0 {
        return None;
    }
    let target = cursor / viewport.scale;
    let (camera, transform) = world
        .query_filtered::<(&Camera, &GlobalTransform), With<WorldCamera>>()
        .single(world)
        .ok()?;
    let point = camera.viewport_to_world_2d(transform, target).ok()?;
    Some(Pos::new(point.x / TILE.0, -point.y / TILE.0))
}

pub fn walkable(world: &World, tile: Pos<Tiles>) -> bool {
    session::me(world)
        .and_then(|me| me.get::<AreaTag>())
        .map(|tag| tag.area)
        .and_then(|id| area::areas().get(id.index()))
        .is_some_and(|area| area.grid.walkable(tile))
}

pub fn enemy_at(world: &mut World, point: Pos<Tiles>) -> Option<Entity> {
    let me = session::me(world).map(|entity| entity.id());
    let mut actors =
        world.query_filtered::<(Entity, &Position, &Hitbox, Option<&Vitals>), With<Actor>>();
    actors.iter(world).find_map(|(entity, at, hitbox, vitals)| {
        if Some(entity) == me || vitals.is_some_and(|vitals| vitals.health <= 0.0) {
            return None;
        }
        hitbox_bounds(at.pos, hitbox.size)
            .contains(point)
            .then_some(entity)
    })
}

/// An actor's hitbox in tile space: feet half a tile below its center, rising by its height.
pub fn hitbox_bounds(pos: Pos<Tiles>, size: Size<Tiles>) -> Rect<Tiles> {
    let feet = pos.y + 0.5;
    Rect::new(Pos::new(pos.x - size.width / 2.0, feet - size.height), size)
}
