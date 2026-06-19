use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use world::math::{Pos, Rect, Tiles};
use world::protocol::{Actor, Hitbox, Position, Vitals};
use world::session;

use crate::render::{TILE, Viewport, WorldCamera};

/// The tile under the cursor, or `None` when the cursor is outside the window.
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

/// The living enemy whose click box contains `point` — feet-anchored, excluding the player.
pub fn enemy_at(world: &mut World, point: Pos<Tiles>) -> Option<Entity> {
    let me = session::me(world).map(|entity| entity.id());
    let mut actors =
        world.query_filtered::<(Entity, &Position, &Hitbox, Option<&Vitals>), With<Actor>>();
    actors.iter(world).find_map(|(entity, at, hitbox, vitals)| {
        if Some(entity) == me || vitals.is_some_and(|vitals| vitals.health <= 0.0) {
            return None;
        }
        let bottom = at.pos.y + 0.5;
        let bounds = Rect::new(
            Pos::new(
                at.pos.x - hitbox.size.width / 2.0,
                bottom - hitbox.size.height,
            ),
            hitbox.size,
        );
        bounds.contains(point).then_some(entity)
    })
}
