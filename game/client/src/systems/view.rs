//! The local player's audiovisual viewpoint: each frame it points the world camera and the audio
//! listener at the local player (the camera clamped to the area's bounds). The camera and audio
//! engines that consume this live in `crate::core`.

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
use crate::core::render::TILE;
use crate::core::render::camera::CameraTarget;
use crate::core::render::present::target_size;

pub struct ViewPlugin;

impl Plugin for ViewPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            track_player.run_if(in_state(crate::GameScene::Playing)),
        );
    }
}

fn track_player(
    me: Res<MyClient>,
    players: Query<(&Owner, &Position, &AreaTag)>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut target: ResMut<CameraTarget>,
    mut listener: ResMut<Listener>,
) {
    let Some(my) = me.0 else {
        return;
    };
    let Some((_, position, tag)) = players.iter().find(|(owner, _, _)| owner.client == my) else {
        return;
    };
    listener.0 = Some(position.pos);
    target.0 = camera_center(position.pos, tag.area, view_half(&window));
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
