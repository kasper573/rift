use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::core::audio::playback::Listener;
use crate::core::content::Content;
use crate::core::interpolate::{Jumps, RenderValueSystems};
use crate::core::math::Pos;
use crate::core::render::camera::WorldCamera;
use crate::core::render::present::{SCALE, target_size};
use crate::core::render::screen::ToScreen;
use crate::core::render::{TILE, snap_to_screen};
use crate::core::tiling::{TileSize, Tiles};
use crate::core::time::Seconds;
use crate::systems::area::{self, AreaId, AreaTag};
use crate::systems::movement::{Position, RenderPosition};
use crate::systems::player::session::MyCharacter;
use crate::systems::screen::ClientScreen;

/// Time the follow camera takes to close half the gap to the player. Smaller is snappier, larger
/// floatier; this is what makes the camera glide after the actor rather than rigidly lock to it.
const CAMERA_HALF_LIFE: Seconds = Seconds(0.09);

pub struct ViewPlugin;

impl Plugin for ViewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FollowCenter>().add_systems(
            Update,
            track_player
                .after(RenderValueSystems)
                .run_if(in_state(ClientScreen::Area)),
        );
    }
}

/// The camera's smoothed world-space center, eased toward the player each frame. Kept here in
/// continuous tiles rather than read back from the pixel-snapped [`Transform`], so smoothing doesn't
/// feed on its own rounding.
#[derive(Resource, Default)]
struct FollowCenter(Option<(Followed, Pos<Tiles>)>);

/// Who the camera follows, where: the camera glides after them only while this stays the same, and
/// cuts to them when it changes, as they jump, cross into another area, or someone else is followed.
#[derive(Clone, Copy, PartialEq)]
struct Followed {
    entity: Entity,
    area: AreaId,
    jumps: Jumps,
}

#[allow(clippy::too_many_arguments)]
fn track_player(
    time: Res<Time>,
    viewpoint: Res<MyCharacter>,
    content: Res<Content>,
    characters: Query<(&RenderPosition, &Position, &AreaTag)>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut camera: Query<&mut Transform, With<WorldCamera>>,
    mut listener: ResMut<Listener>,
    mut follow: ResMut<FollowCenter>,
) {
    let Some(seen) = viewpoint.0 else {
        return;
    };
    let Ok((render, server, tag)) = characters.get(seen) else {
        return;
    };
    let at = render.0;
    listener.0 = Some(at);
    let Some(target) = camera_center(&content, at, tag.area, view_half(&window)) else {
        return;
    };
    let followed = Followed {
        entity: seen,
        area: tag.area,
        jumps: server.jumps,
    };
    let center = match follow.0 {
        Some((before, prev)) if before == followed => {
            let caught = 1.0 - 0.5_f32.powf(time.delta_secs() / CAMERA_HALF_LIFE.0);
            prev.lerp(target, caught)
        }
        _ => target,
    };
    follow.0 = Some((followed, center));
    if let Ok(mut transform) = camera.single_mut() {
        let screen = snap_to_screen(center.to_screen());
        transform.translation.x = screen.x;
        transform.translation.y = screen.y;
    }
}

fn camera_center(
    content: &Content,
    at: Pos<Tiles>,
    area_id: AreaId,
    half: Vec2,
) -> Option<Pos<Tiles>> {
    let area = area::load(content, area_id);
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
