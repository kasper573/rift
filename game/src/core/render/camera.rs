use crate::core::math::Pos;
use crate::core::tiling::Tiles;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use super::present::Viewport;
use crate::core::render::screen::{ToScreen, ToTile};

#[derive(Component)]
pub(crate) struct WorldCamera;

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

pub fn tile_to_window(world: &mut World, tile: Pos<Tiles>) -> Option<Vec2> {
    let viewport = *world.resource::<Viewport>();
    let (camera, transform) = world
        .query_filtered::<(&Camera, &GlobalTransform), With<WorldCamera>>()
        .single(world)
        .ok()?;
    let point = camera
        .world_to_viewport(transform, tile.to_screen().extend(0.0))
        .ok()?;
    Some(point * viewport.scale)
}
