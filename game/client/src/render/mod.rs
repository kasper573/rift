//! Rendering: the world draws to a fixed-zoom offscreen texture ([`present`]) that a [`camera`]
//! follows, with the map [`tiles`], replicated [`actors`], and world-space [`overlay`]s drawn into it.

pub mod actors;
pub mod camera;
pub mod overlay;
pub mod present;
pub mod screen;
pub mod tiles;

pub use actors::Animator;
pub use camera::cursor_tile;

use bevy::prelude::*;
use bevy::sprite_render::Material2dPlugin;
use world::area;
use world::core::math::{Pos, WorldPx};
use world::core::tiling::Tiles;

use crate::render::screen::ToScreen;

pub const TILE: WorldPx = WorldPx(16.0);

pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(Material2dPlugin::<present::Present>::default())
            .init_resource::<Animator>()
            .init_resource::<tiles::SpawnedArea>()
            .init_resource::<present::Viewport>()
            .add_systems(Startup, (present::setup, overlay::setup))
            .add_observer(actors::attach_sprite)
            .add_systems(Update, (present::match_display, present::fit).chain())
            .add_systems(
                Update,
                (
                    actors::sync_actors,
                    camera::follow_camera,
                    tiles::spawn_area_tiles,
                    tiles::animate_tiles,
                    present::dead_tint,
                    overlay::healthbar,
                    overlay::update_tile_highlight,
                )
                    .run_if(in_state(crate::GameScene::Playing)),
            );
    }
}

fn sprite_transform(pos: Pos<Tiles>, z: f32) -> Transform {
    Transform::from_translation(pos.to_screen().extend(z))
}

fn dynamic_z(area: &area::Area, base: f32, y: Tiles) -> f32 {
    base + (y + Tiles(1.0)).ratio(Tiles(area.size.height + 2.0))
}

fn atlas_rect(region: world::core::math::Rect<WorldPx>) -> Rect {
    Rect::new(
        region.min().x,
        region.min().y,
        region.max().x,
        region.max().y,
    )
}
