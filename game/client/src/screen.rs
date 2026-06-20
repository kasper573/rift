//! Projection between tile space and the rendered screen: scale by the tile size and flip the y-axis.

use bevy::math::Vec2;
use world::math::{Pos, Size};
use world::tiling::Tiles;

use crate::render::TILE;

pub fn to_screen(pos: Pos<Tiles>) -> Vec2 {
    Vec2::new(pos.x * TILE.0, -pos.y * TILE.0)
}

pub fn to_screen_size(size: Size<Tiles>) -> Vec2 {
    Vec2::new(size.width * TILE.0, size.height * TILE.0)
}

pub fn to_tile(screen: Vec2) -> Pos<Tiles> {
    Pos::new(screen.x / TILE.0, -screen.y / TILE.0)
}
