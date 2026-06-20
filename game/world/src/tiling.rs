//! Tile coordinates and tile↔pixel conversion.

use serde::{Deserialize, Serialize};

use crate::math::{Pos, Rect, Size, WorldPx};
use crate::time::Seconds;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct Tiles(pub f32);

pub type CellPos = euclid::default::Point2D<i32>;
pub type GridSize = euclid::default::Size2D<u32>;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct TilesPerSec(pub Tiles);

impl std::ops::Add for Tiles {
    type Output = Tiles;
    fn add(self, other: Tiles) -> Tiles {
        Tiles(self.0 + other.0)
    }
}

impl std::ops::Sub for Tiles {
    type Output = Tiles;
    fn sub(self, other: Tiles) -> Tiles {
        Tiles(self.0 - other.0)
    }
}

impl std::ops::SubAssign for Tiles {
    fn sub_assign(&mut self, other: Tiles) {
        self.0 -= other.0;
    }
}

impl std::ops::Mul<f32> for TilesPerSec {
    type Output = TilesPerSec;
    fn mul(self, factor: f32) -> TilesPerSec {
        TilesPerSec(Tiles(self.0.0 * factor))
    }
}

impl std::ops::Mul<Seconds> for TilesPerSec {
    type Output = Tiles;
    fn mul(self, time: Seconds) -> Tiles {
        Tiles(self.0.0 * time.0)
    }
}

// Tile coordinates, single source of truth. An integer `CellPos` names a tile; its
// center sits at the same numbers in continuous `Tiles` space, and the tile fills the
// unit square around it. Nothing outside this section may add or subtract half-tiles:
// "tile 0,0" is where you stand, and ".5" is only the transient state between tiles.

pub fn tile_center(cell: CellPos) -> Pos<Tiles> {
    Pos::new(cell.x as f32, cell.y as f32)
}

pub fn tile_at(p: Pos<Tiles>) -> CellPos {
    CellPos::new(p.x.round() as i32, p.y.round() as i32)
}

pub fn snap_to_tile(p: Pos<Tiles>) -> Pos<Tiles> {
    tile_center(tile_at(p))
}

pub fn on_center(p: Pos<Tiles>) -> bool {
    let resting = |c: f32| (c - c.round()).abs() < 1e-3;
    resting(p.x) && resting(p.y)
}

pub fn tile_bounds(cell: CellPos) -> Rect<Tiles> {
    Rect::new(
        Pos::new(cell.x as f32 - 0.5, cell.y as f32 - 0.5),
        Size::splat(1.0),
    )
}

pub fn grid_bounds(width: Tiles, height: Tiles) -> Rect<Tiles> {
    Rect::new(Pos::new(-0.5, -0.5), Size::new(width.0, height.0))
}

pub fn tiles_in(rect: Rect<Tiles>) -> impl Iterator<Item = CellPos> {
    let min = tile_at(rect.min());
    let max = tile_at(rect.max());
    (min.y..=max.y).flat_map(move |y| (min.x..=max.x).map(move |x| CellPos::new(x, y)))
}

#[derive(Clone, Copy)]
pub struct PixelsPerTile {
    x: euclid::Scale<f32, WorldPx, Tiles>,
    y: euclid::Scale<f32, WorldPx, Tiles>,
}

impl PixelsPerTile {
    pub fn new(tile_width: WorldPx, tile_height: WorldPx) -> PixelsPerTile {
        PixelsPerTile {
            x: euclid::Scale::new(1.0 / tile_width.0.max(1.0)),
            y: euclid::Scale::new(1.0 / tile_height.0.max(1.0)),
        }
    }

    // Source maps put a tile's origin at its top-left corner; ours is the center, half a tile in.
    pub fn point(self, p: Pos<WorldPx>) -> Pos<Tiles> {
        Pos::new(
            self.x.transform_point(p).x - 0.5,
            self.y.transform_point(p).y - 0.5,
        )
    }

    pub fn rect(self, r: Rect<WorldPx>) -> Rect<Tiles> {
        Rect::new(
            self.point(r.origin),
            Size::new(r.size.width * self.x.get(), r.size.height * self.y.get()),
        )
    }

    pub fn tile_center(self, p: Pos<WorldPx>) -> Pos<Tiles> {
        snap_to_tile(self.point(p))
    }
}
