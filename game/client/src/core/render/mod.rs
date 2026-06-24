//! The render pipeline: the world draws to a fixed-zoom offscreen texture ([`present`]) a [`camera`]
//! follows, in screen space ([`screen`]), with a generic [`present::ScreenTint`] hook. All actual
//! drawing — actors, tiles, the health bar, the death wash — is bespoke and lives in the feature
//! modules under `crate::systems`, sharing the [`Animator`] clock and the transform helpers here.

pub mod camera;
pub mod present;
pub mod screen;

pub use camera::cursor_tile;

use std::collections::HashMap;

use bevy::prelude::*;
use bevy::sprite_render::Material2dPlugin;
use world::core::math::{Pos, WorldPx};
use world::core::tiling::Tiles;
use world::core::time::Seconds;

use self::screen::ToScreen;

pub const TILE: WorldPx = WorldPx(16.0);

pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(Material2dPlugin::<present::Present>::default())
            .init_resource::<Animator>()
            .init_resource::<camera::CameraTarget>()
            .init_resource::<present::ScreenTint>()
            .init_resource::<present::Viewport>()
            .add_systems(Startup, present::setup)
            .add_systems(Update, (present::match_display, present::fit).chain())
            .add_systems(
                Update,
                (camera::follow_camera, present::apply_tint)
                    .run_if(in_state(crate::GameScene::Playing)),
            );
    }
}

/// Per-entity animation clock: how long an entity has held its current state (an opaque `u64` key the
/// caller supplies, e.g. an action discriminant), so the renderer and the audio cue scheduler sample
/// the same frame. The key is opaque on purpose — this stays free of any game-specific type.
#[derive(Resource, Default)]
pub struct Animator {
    anchors: HashMap<Entity, (u64, Seconds)>,
}

impl Animator {
    pub fn elapsed(&mut self, entity: Entity, state: u64, time: Seconds) -> Seconds {
        match self.anchors.get(&entity) {
            Some(&(seen, start)) if seen == state => time - start,
            _ => {
                self.anchors.insert(entity, (state, time));
                Seconds(0.0)
            }
        }
    }

    pub fn retain(&mut self, mut alive: impl FnMut(Entity) -> bool) {
        self.anchors.retain(|entity, _| alive(*entity));
    }
}

pub(crate) fn sprite_transform(pos: Pos<Tiles>, z: f32) -> Transform {
    Transform::from_translation(pos.to_screen().extend(z))
}

pub(crate) fn dynamic_z(area_height: f32, base: f32, y: Tiles) -> f32 {
    base + (y + Tiles(1.0)).ratio(Tiles(area_height + 2.0))
}

pub(crate) fn atlas_rect(region: world::core::math::Rect<WorldPx>) -> Rect {
    Rect::new(
        region.min().x,
        region.min().y,
        region.max().x,
        region.max().y,
    )
}
