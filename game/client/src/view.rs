use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use world::area;
use world::math::Pos;
use world::protocol::{Actor, AreaTag, Hitbox, Position, Vitals};
use world::session;
use world::tiling::{TilePos, Tiles};

use crate::render::{Viewport, WorldCamera};
use crate::screen::ToTile;

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
    Some(point.to_tile())
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
        if Some(entity) == me || vitals.is_some_and(Vitals::is_dead) {
            return None;
        }
        at.pos.hitbox(hitbox.size).contains(point).then_some(entity)
    })
}
