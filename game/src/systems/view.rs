use crate::core::assets::AssetService;
use crate::core::math::Pos;
use crate::core::tiling::{TilePos, TileSize, Tiles};
use crate::core::time::Seconds;
use crate::systems::area::{self, AreaTag};
use crate::systems::player::Owner;
use crate::systems::player::session::MyClient;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::core::render::camera::WorldCamera;
use crate::core::render::present::{SCALE, target_size};
use crate::core::render::screen::ToScreen;
use crate::core::render::{TILE, snap_to_screen};
use crate::core::sfx::playback::Listener;
use crate::systems::movement::RenderPosition;

/// Time the follow camera takes to close half the gap to the player. Smaller is snappier, larger
/// floatier; this is what makes the camera glide after the actor rather than rigidly lock to it.
const CAMERA_HALF_LIFE: Seconds = Seconds(0.09);

/// A target this far from the camera is an area change or respawn, not a walk, so the camera cuts
/// straight to it instead of gliding across the whole map.
const CAMERA_CUT: Tiles = Tiles(6.0);

pub struct ViewPlugin;

impl Plugin for ViewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FollowCenter>().add_systems(
            Update,
            track_player.run_if(in_state(crate::systems::scene::Scene::Area)),
        );
    }
}

/// The camera's smoothed world-space center, eased toward the player each frame. Kept here in
/// continuous tiles rather than read back from the pixel-snapped [`Transform`], so smoothing doesn't
/// feed on its own rounding.
#[derive(Resource, Default)]
struct FollowCenter(Option<Pos<Tiles>>);

#[allow(clippy::too_many_arguments)]
fn track_player(
    time: Res<Time>,
    me: Res<MyClient>,
    service: Res<AssetService>,
    players: Query<(&Owner, &RenderPosition, &AreaTag)>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut camera: Query<&mut Transform, With<WorldCamera>>,
    mut listener: ResMut<Listener>,
    mut follow: ResMut<FollowCenter>,
) {
    let Some(my) = me.0 else {
        return;
    };
    let Some((_, render, tag)) = players.iter().find(|(owner, ..)| owner.client == my) else {
        return;
    };
    let at = render.0;
    listener.0 = Some(at);
    let Some(target) = camera_center(&service, at, tag.area, view_half(&window)) else {
        return;
    };
    let center = match follow.0 {
        Some(prev) if prev.distance(target) <= CAMERA_CUT => {
            let caught = 1.0 - 0.5_f32.powf(time.delta_secs() / CAMERA_HALF_LIFE.0);
            prev.lerp(target, caught)
        }
        _ => target,
    };
    follow.0 = Some(center);
    if let Ok(mut transform) = camera.single_mut() {
        let screen = snap_to_screen(center.to_screen());
        transform.translation.x = screen.x;
        transform.translation.y = screen.y;
    }
}

fn camera_center(
    service: &AssetService,
    at: Pos<Tiles>,
    area_id: crate::systems::area::Id,
    half: Vec2,
) -> Option<Pos<Tiles>> {
    let area = service.resolve(area_id.get().map, area::build_area);
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
