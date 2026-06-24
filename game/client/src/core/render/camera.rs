//! The world camera: its marker, cursor un-projection into tile space (for the input gestures), and a
//! generic follow that tracks a [`CameraTarget`]. Deciding *what* to track (and clamping it to the
//! area) is a game concern and lives in `crate::systems::view`.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use world::core::math::Pos;
use world::core::tiling::Tiles;

use super::present::Viewport;
use crate::core::render::screen::{ToScreen, ToTile};

#[derive(Component)]
pub(crate) struct WorldCamera;

/// The point the camera centres on, in tile space; a game system keeps it where the view should be.
/// Absent ⇒ the camera holds its last position.
#[derive(Resource, Default)]
pub struct CameraTarget(pub Option<Pos<Tiles>>);

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

pub(super) fn follow_camera(
    target: Res<CameraTarget>,
    mut camera: Query<&mut Transform, With<WorldCamera>>,
) {
    let Some(center) = target.0 else {
        return;
    };
    if let Ok(mut transform) = camera.single_mut() {
        let at = center.to_screen();
        transform.translation.x = at.x;
        transform.translation.y = at.y;
    }
}
