//! The render pipeline: the world draws to a fixed-zoom offscreen texture ([`present`]) a [`camera`]
//! follows, in screen space ([`screen`]), with a generic [`present::ScreenTint`] hook. All actual
//! drawing — actors, tiles, the health bar, the death wash — is bespoke and lives in the feature
//! modules under `crate::systems`, sharing the [`Animator`] clock and the [`bevy_tiled`] tile
//! projection and transform helpers re-exported below.

pub mod camera;
pub mod present;
pub mod screen;

pub use camera::cursor_tile;

use std::collections::HashMap;

use bevy::prelude::*;
use bevy::sprite_render::Material2dPlugin;
use world::core::time::Seconds;

pub use bevy_tiled::{TILE, atlas_rect, dynamic_z, sprite_transform};

pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(Material2dPlugin::<present::Present>::default())
            .init_resource::<Animator>()
            .init_resource::<present::ScreenTint>()
            .init_resource::<present::Viewport>()
            .add_systems(Startup, present::setup)
            .add_systems(Update, (present::match_display, present::fit).chain())
            .add_systems(
                Update,
                present::apply_tint.run_if(in_state(crate::GameScene::Playing)),
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
