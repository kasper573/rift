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

pub use screen::ToScreen;

pub const TILE: WorldPx = WorldPx(16.0);

pub fn sprite_transform(pos: Pos<Tiles>, z: f32) -> Transform {
    Transform::from_translation(pos.to_screen().extend(z))
}

pub fn snap_to_screen(at: Vec2) -> Vec2 {
    (at * present::SCALE).round() / present::SCALE
}

pub fn dynamic_z(area_height: f32, base: f32, y: Tiles) -> f32 {
    base + (y + Tiles(1.0)).ratio(Tiles(area_height + 2.0))
}

pub fn atlas_rect(region: world::core::math::Rect<WorldPx>) -> Rect {
    Rect::new(
        region.min().x,
        region.min().y,
        region.max().x,
        region.max().y,
    )
}

pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(Material2dPlugin::<present::Present>::default())
            .init_resource::<Animator>()
            .init_resource::<present::ScreenTint>()
            .init_resource::<present::Viewport>()
            .add_systems(Startup, present::setup)
            .add_systems(Update, (present::match_display, present::fit).chain())
            .add_systems(Update, present::apply_tint);
    }
}

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
