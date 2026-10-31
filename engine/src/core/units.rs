use std::ops::Mul;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::math::{Offset, Pos, Rect, Size, WorldPx};
use crate::core::time::Seconds;

#[derive(
    JsonSchema,
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
#[schemars(extend("x-unit" = "m"))]
pub struct Meters(pub f32);

/// An amount of memory.
#[derive(
    Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord,
)]
pub struct Bytes(pub u64);

impl Bytes {
    pub fn megabytes(self) -> f64 {
        self.0 as f64 / (1024.0 * 1024.0)
    }
}

/// Stops of light: each one doubles it.
#[derive(
    JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd,
)]
#[schemars(extend("x-unit" = "EV"))]
pub struct Stops(pub f32);

impl Stops {
    pub fn is_zero(&self) -> bool {
        self.0 == 0.0
    }
}

pub type CellPos = euclid::default::Point2D<i32>;
pub type GridSize = euclid::default::Size2D<u32>;

/// The corner of an area's ground: cells are centred on whole metres, so the cell `(0, 0)` and the
/// ground begin half a metre before the origin.
pub const GROUND_ORIGIN: Pos<Meters> = Pos::new(-0.5, -0.5);

#[derive(
    JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd,
)]
pub struct MetersPerSec(pub Meters);

impl Meters {
    pub fn ratio(self, other: Meters) -> f32 {
        self.0 / other.0
    }

    pub fn is_zero(&self) -> bool {
        self.0 == 0.0
    }
}

impl Mul<f32> for MetersPerSec {
    type Output = MetersPerSec;
    fn mul(self, factor: f32) -> MetersPerSec {
        MetersPerSec(Meters(self.0.0 * factor))
    }
}

impl Mul<Seconds> for MetersPerSec {
    type Output = Meters;
    fn mul(self, time: Seconds) -> Meters {
        Meters(self.0.0 * time.0)
    }
}

/// A metre cell of an area's ground, the grain its terrain is painted and sloped at.
pub trait Cell {
    fn center(self) -> Pos<Meters>;
    fn bounds(self) -> Rect<Meters>;
    fn index(self, size: GridSize) -> Option<usize>;
}

impl Cell for CellPos {
    fn center(self) -> Pos<Meters> {
        Pos::new(self.x as f32, self.y as f32)
    }

    fn bounds(self) -> Rect<Meters> {
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
}

pub trait MeterPos {
    fn distance(self, other: Pos<Meters>) -> Meters;
    fn toward(self, target: Pos<Meters>, by: Meters) -> Pos<Meters>;
    fn hitbox(self, size: Size<Meters>) -> Rect<Meters>;
}

impl MeterPos for Pos<Meters> {
    fn distance(self, other: Pos<Meters>) -> Meters {
        Meters(self.distance_to(other))
    }

    fn toward(self, target: Pos<Meters>, by: Meters) -> Pos<Meters> {
        self + (target - self).normalize() * by.0
    }

    fn hitbox(self, size: Size<Meters>) -> Rect<Meters> {
        Rect::new(
            self + Offset::new(-size.width / 2.0, 0.5 - size.height),
            size,
        )
    }
}

pub trait MeterSize {
    fn bounds(self) -> Rect<Meters>;
    fn grid(self) -> GridSize;
}

impl MeterSize for Size<Meters> {
    fn bounds(self) -> Rect<Meters> {
        Rect::new(GROUND_ORIGIN, self)
    }

    fn grid(self) -> GridSize {
        GridSize::new(self.width as u32, self.height as u32)
    }
}

pub trait GridDims {
    fn cells(self) -> impl Iterator<Item = CellPos>;
    /// The cell under `at`, none off the grid.
    fn cell_under(self, at: Pos<Meters>) -> Option<CellPos>;
}

impl GridDims for GridSize {
    fn cells(self) -> impl Iterator<Item = CellPos> {
        let (w, h) = (self.width as i32, self.height as i32);
        (0..h).flat_map(move |y| (0..w).map(move |x| CellPos::new(x, y)))
    }

    fn cell_under(self, at: Pos<Meters>) -> Option<CellPos> {
        let cell = CellPos::new(at.x.round() as i32, at.y.round() as i32);
        cell.index(self).map(|_| cell)
    }
}

#[derive(Clone, Copy)]
pub struct PixelsPerTile {
    x: euclid::Scale<f32, WorldPx, Meters>,
    y: euclid::Scale<f32, WorldPx, Meters>,
}

impl PixelsPerTile {
    pub fn new(tile_size: Size<WorldPx>) -> PixelsPerTile {
        PixelsPerTile {
            x: euclid::Scale::new(1.0 / tile_size.width.max(1.0)),
            y: euclid::Scale::new(1.0 / tile_size.height.max(1.0)),
        }
    }

    pub fn point(self, p: Pos<WorldPx>) -> Pos<Meters> {
        Pos::new(
            self.x.transform_point(p).x - 0.5,
            self.y.transform_point(p).y - 0.5,
        )
    }

    pub fn rect(self, r: Rect<WorldPx>) -> Rect<Meters> {
        Rect::new(
            self.point(r.origin),
            Size::new(r.size.width * self.x.get(), r.size.height * self.y.get()),
        )
    }
}
