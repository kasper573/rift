use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use super::present::PresentScale;
use super::screen::{ToScreen, ToTile};
use crate::core::gui;
use crate::core::math::Pos;
use crate::core::units::Meters;

#[derive(Component)]
pub(crate) struct WorldCamera;

pub fn cursor_tile(world: &mut World) -> Option<Pos<Meters>> {
    if gui::pointer_claimed(world) {
        return None;
    }
    let cursor = world
        .query_filtered::<&Window, With<PrimaryWindow>>()
        .single(world)
        .ok()?
        .cursor_position()?;
    let scale = *world.resource::<PresentScale>();
    if scale.0 <= 0.0 {
        return None;
    }
    let target = cursor / scale.0;
    let (camera, transform) = world
        .query_filtered::<(&Camera, &GlobalTransform), With<WorldCamera>>()
        .single(world)
        .ok()?;
    let point = camera.viewport_to_world_2d(transform, target).ok()?;
    Some(point.to_tile())
}

pub fn tile_to_window(world: &mut World, tile: Pos<Meters>) -> Option<Vec2> {
    let scale = *world.resource::<PresentScale>();
    let (camera, transform) = world
        .query_filtered::<(&Camera, &GlobalTransform), With<WorldCamera>>()
        .single(world)
        .ok()?;
    project(scale, camera, transform, tile)
}

fn project(
    scale: PresentScale,
    camera: &Camera,
    transform: &GlobalTransform,
    tile: Pos<Meters>,
) -> Option<Vec2> {
    let point = camera
        .world_to_viewport(transform, tile.to_screen().extend(0.0))
        .ok()?;
    Some(point * scale.0)
}
