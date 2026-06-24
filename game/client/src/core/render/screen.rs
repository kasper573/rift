//! Projection between tile space and the rendered screen: scale by the tile size and flip the y-axis.

use bevy::math::Vec2;
use world::core::math::{Pos, Size};
use world::core::tiling::Tiles;

use crate::core::render::TILE;

pub trait ToScreen {
    fn to_screen(self) -> Vec2;
}

impl ToScreen for Pos<Tiles> {
    fn to_screen(self) -> Vec2 {
        Vec2::new(self.x * TILE.0, -self.y * TILE.0)
    }
}

impl ToScreen for Size<Tiles> {
    fn to_screen(self) -> Vec2 {
        Vec2::new(self.width * TILE.0, self.height * TILE.0)
    }
}

pub trait ToTile {
    fn to_tile(self) -> Pos<Tiles>;
}

impl ToTile for Vec2 {
    fn to_tile(self) -> Pos<Tiles> {
        Pos::new(self.x / TILE.0, -self.y / TILE.0)
    }
}
