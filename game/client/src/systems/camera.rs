//! Makes the world camera follow the local player, clamped to the area's bounds.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use world::core::math::Pos;
use world::core::table::Id;
use world::core::tiling::{TileSize, Tiles};
use world::systems::area::{self, AreaDef, AreaTag};
use world::systems::movement::Position;
use world::systems::player::Owner;
use world::systems::player::session::MyClient;

use crate::core::render::TILE;
use crate::core::render::camera::WorldCamera;
use crate::core::render::present::target_size;
use crate::core::render::screen::ToScreen;

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            follow_camera.run_if(in_state(crate::GameScene::Playing)),
        );
    }
}

fn follow_camera(
    me: Res<MyClient>,
    players: Query<(&Owner, &Position, &AreaTag)>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut camera: Query<&mut Transform, With<WorldCamera>>,
) {
    let Some(my) = me.0 else {
        return;
    };
    let Some((_, position, tag)) = players.iter().find(|(owner, _, _)| owner.client == my) else {
        return;
    };
    let Some(center) = camera_center(position.pos, tag.area, view_half(&window)) else {
        return;
    };
    if let Ok(mut transform) = camera.single_mut() {
        let p = center.to_screen();
        transform.translation.x = p.x;
        transform.translation.y = p.y;
    }
}

fn camera_center(at: Pos<Tiles>, area_id: Id<AreaDef>, half: Vec2) -> Option<Pos<Tiles>> {
    let area = area::areas().get(area_id.index())?;
    let bounds = area.size.bounds();
    let lo = Pos::new(bounds.min().x + half.x, bounds.min().y + half.y);
    let hi = Pos::new(
        (bounds.max().x - half.x).max(lo.x),
        (bounds.max().y - half.y).max(lo.y),
    );
    Some(snap(at.clamp(lo, hi)))
}

fn view_half(window: &Window) -> Vec2 {
    let (w, h) = target_size(window);
    Vec2::new(0.5 * w as f32 / TILE.0, 0.5 * h as f32 / TILE.0)
}

fn snap(p: Pos<Tiles>) -> Pos<Tiles> {
    let axis = |t: f32| (t * TILE.0).round() / TILE.0;
    Pos::new(axis(p.x), axis(p.y))
}
