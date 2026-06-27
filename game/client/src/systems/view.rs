use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use world::core::math::Pos;
use world::core::table::Id;
use world::core::tiling::{TileSize, Tiles};
use world::systems::area::{self, AreaDef, AreaTag};
use world::systems::movement::Position;
use world::systems::player::Owner;
use world::systems::player::session::MyClient;

use crate::core::audio::Listener;
use crate::core::render::camera::WorldCamera;
use crate::core::render::present::{SCALE, target_size};
use crate::core::render::screen::ToScreen;
use crate::core::render::{TILE, snap_to_screen};

pub struct ViewPlugin;

impl Plugin for ViewPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, track_player.run_if(in_state(crate::Scene::Area)));
    }
}

fn track_player(
    me: Res<MyClient>,
    players: Query<(&Owner, &Position, &AreaTag)>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut camera: Query<&mut Transform, With<WorldCamera>>,
    mut listener: ResMut<Listener>,
) {
    let Some(my) = me.0 else {
        return;
    };
    let Some((_, position, tag)) = players.iter().find(|(owner, _, _)| owner.client == my) else {
        return;
    };
    listener.0 = Some(position.pos);
    let Some(center) = camera_center(position.pos, tag.area, view_half(&window)) else {
        return;
    };
    if let Ok(mut transform) = camera.single_mut() {
        let at = snap_to_screen(center.to_screen());
        transform.translation.x = at.x;
        transform.translation.y = at.y;
    }
}

fn camera_center(at: Pos<Tiles>, area_id: Id<AreaDef>, half: Vec2) -> Option<Pos<Tiles>> {
    let area = area::get(area_id)?;
    let bounds = area.size.bounds();
    let lo = Pos::new(bounds.min().x + half.x, bounds.min().y + half.y);
    let hi = Pos::new(
        (bounds.max().x - half.x).max(lo.x),
        (bounds.max().y - half.y).max(lo.y),
    );
    Some(at.clamp(lo, hi))
}

fn view_half(window: &Window) -> Vec2 {
    let (w, h) = target_size(window);
    let per_tile = TILE.0 * SCALE;
    Vec2::new(0.5 * w as f32 / per_tile, 0.5 * h as f32 / per_tile)
}
