//! The world camera marker and cursor un-projection into tile space (for the input gestures). Moving
//! the camera (following the player, clamped to the area) is a game concern and lives in
//! `crate::systems::view`.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use world::core::math::Pos;
use world::core::tiling::Tiles;

use super::present::Viewport;
use crate::core::render::screen::ToTile;

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
