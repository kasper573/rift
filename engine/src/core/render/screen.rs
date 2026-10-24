use bevy::prelude::Vec2;

use super::TILE;
use crate::core::math::{Pos, Size};
use crate::core::units::Meters;

pub trait ToScreen {
    fn to_screen(self) -> Vec2;
}

impl ToScreen for Pos<Meters> {
    fn to_screen(self) -> Vec2 {
        Vec2::new(self.x * TILE.0, -self.y * TILE.0)
    }
}

impl ToScreen for Size<Meters> {
    fn to_screen(self) -> Vec2 {
        Vec2::new(self.width * TILE.0, self.height * TILE.0)
    }
}

pub trait ToTile {
    fn to_tile(self) -> Pos<Meters>;
}

impl ToTile for Vec2 {
    fn to_tile(self) -> Pos<Meters> {
        Pos::new(self.x / TILE.0, -self.y / TILE.0)
    }
}
