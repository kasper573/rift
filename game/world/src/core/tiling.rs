use serde::{Deserialize, Serialize};

use crate::core::math::{Offset, Pos, Rect, Size, WorldPx};
use crate::core::time::Seconds;

#[derive(
    Serialize,
    Deserialize,
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    PartialOrd,
    derive_more::Add,
    derive_more::Sub,
    derive_more::SubAssign,
)]
pub struct Tiles(pub f32);

pub type CellPos = euclid::default::Point2D<i32>;
pub type GridSize = euclid::default::Size2D<u32>;

pub const NEIGHBORS_4: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
pub const NEIGHBORS_8: [(i32, i32); 8] = [
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (1, 1),
    (1, -1),
    (-1, 1),
    (-1, -1),
];

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct TilesPerSec(pub Tiles);

impl Tiles {
    pub fn ratio(self, other: Tiles) -> f32 {
        self.0 / other.0
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

pub trait Cell {
    fn center(self) -> Pos<Tiles>;
    fn bounds(self) -> Rect<Tiles>;
    fn index(self, size: GridSize) -> Option<usize>;
    fn step(self, offset: (i32, i32)) -> CellPos;
}

impl Cell for CellPos {
    fn center(self) -> Pos<Tiles> {
        Pos::new(self.x as f32, self.y as f32)
    }

    fn bounds(self) -> Rect<Tiles> {
        Rect::new(
            Pos::new(self.x as f32 - 0.5, self.y as f32 - 0.5),
            Size::splat(1.0),
        )
    }

    fn index(self, size: GridSize) -> Option<usize> {
        let (w, h) = (size.width as i32, size.height as i32);
        let inside = self.x >= 0 && self.y >= 0 && self.x < w && self.y < h;
        inside.then(|| (self.y * w + self.x) as usize)
    }

    fn step(self, (dx, dy): (i32, i32)) -> CellPos {
        CellPos::new(self.x + dx, self.y + dy)
    }
}

pub trait TilePos {
    fn cell(self) -> CellPos;
    fn snap(self) -> Pos<Tiles>;
    fn on_center(self) -> bool;
    fn distance(self, other: Pos<Tiles>) -> Tiles;
    fn toward(self, target: Pos<Tiles>, by: Tiles) -> Pos<Tiles>;
    fn hitbox(self, size: Size<Tiles>) -> Rect<Tiles>;
}

impl TilePos for Pos<Tiles> {
    fn cell(self) -> CellPos {
        CellPos::new(self.x.round() as i32, self.y.round() as i32)
    }

    fn snap(self) -> Pos<Tiles> {
        self.cell().center()
    }

    fn on_center(self) -> bool {
        let resting = |c: f32| (c - c.round()).abs() < 1e-3;
        resting(self.x) && resting(self.y)
    }

    fn distance(self, other: Pos<Tiles>) -> Tiles {
        Tiles(self.distance_to(other))
    }

    fn toward(self, target: Pos<Tiles>, by: Tiles) -> Pos<Tiles> {
        self + (target - self).normalize() * by.0
    }

    fn hitbox(self, size: Size<Tiles>) -> Rect<Tiles> {
        Rect::new(
            self + Offset::new(-size.width / 2.0, 0.5 - size.height),
            size,
        )
    }
}

pub trait TileRect {
    fn tiles(self) -> impl Iterator<Item = CellPos>;
}

impl TileRect for Rect<Tiles> {
    fn tiles(self) -> impl Iterator<Item = CellPos> {
        let min = self.min().cell();
        let max = self.max().cell();
        (min.y..=max.y).flat_map(move |y| (min.x..=max.x).map(move |x| CellPos::new(x, y)))
    }
}

pub trait TileSize {
    fn bounds(self) -> Rect<Tiles>;
    fn grid(self) -> GridSize;
}

impl TileSize for Size<Tiles> {
    fn bounds(self) -> Rect<Tiles> {
        Rect::new(Pos::new(-0.5, -0.5), self)
    }

    fn grid(self) -> GridSize {
        GridSize::new(self.width as u32, self.height as u32)
    }
}

pub trait GridDims {
    fn cells(self) -> impl Iterator<Item = CellPos>;
}

impl GridDims for GridSize {
    fn cells(self) -> impl Iterator<Item = CellPos> {
        let (w, h) = (self.width as i32, self.height as i32);
        (0..h).flat_map(move |y| (0..w).map(move |x| CellPos::new(x, y)))
    }
}

#[derive(Clone, Copy)]
pub struct PixelsPerTile {
    x: euclid::Scale<f32, WorldPx, Tiles>,
    y: euclid::Scale<f32, WorldPx, Tiles>,
}

impl PixelsPerTile {
    pub fn new(tile_size: Size<WorldPx>) -> PixelsPerTile {
        PixelsPerTile {
            x: euclid::Scale::new(1.0 / tile_size.width.max(1.0)),
            y: euclid::Scale::new(1.0 / tile_size.height.max(1.0)),
        }
    }

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
        self.point(p).snap()
    }
}
