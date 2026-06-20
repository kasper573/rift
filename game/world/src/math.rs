//! Euclid geometry with unit tags to prevent type confusion.

use std::f32::consts::FRAC_1_SQRT_2;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct Tiles(pub f32);

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct WorldPx(pub f32);

pub type Pos<U> = euclid::Point2D<f32, U>;
pub type Offset<U> = euclid::Vector2D<f32, U>;
pub type Size<U> = euclid::Size2D<f32, U>;
pub type Rect<U> = euclid::Rect<f32, U>;
pub type CellPos = euclid::default::Point2D<i32>;
pub type GridSize = euclid::default::Size2D<u32>;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct TilesPerSec(pub Tiles);

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct Seconds(pub f32);

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct Millis(pub f32);

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct PlaybackRate(pub f32);

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct Hertz(pub f32);

impl Hertz {
    pub fn period(self) -> std::time::Duration {
        std::time::Duration::from_secs_f32(1.0 / self.0)
    }
}

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

impl std::ops::Add for Seconds {
    type Output = Seconds;
    fn add(self, other: Seconds) -> Seconds {
        Seconds(self.0 + other.0)
    }
}

impl std::ops::Sub for Seconds {
    type Output = Seconds;
    fn sub(self, other: Seconds) -> Seconds {
        Seconds(self.0 - other.0)
    }
}

impl std::ops::Div<PlaybackRate> for Seconds {
    type Output = Seconds;
    fn div(self, rate: PlaybackRate) -> Seconds {
        Seconds(self.0 / rate.0)
    }
}

impl std::ops::Add for Millis {
    type Output = Millis;
    fn add(self, other: Millis) -> Millis {
        Millis(self.0 + other.0)
    }
}

impl std::ops::AddAssign for Millis {
    fn add_assign(&mut self, other: Millis) {
        self.0 += other.0;
    }
}

impl std::ops::Sub for Millis {
    type Output = Millis;
    fn sub(self, other: Millis) -> Millis {
        Millis(self.0 - other.0)
    }
}

impl std::ops::SubAssign for Millis {
    fn sub_assign(&mut self, other: Millis) {
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

impl Millis {
    pub fn seconds(self) -> Seconds {
        Seconds(self.0 / 1000.0)
    }
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

    pub fn point(self, p: Pos<WorldPx>) -> Pos<Tiles> {
        Pos::new(self.x.transform_point(p).x, self.y.transform_point(p).y)
    }

    pub fn rect(self, r: Rect<WorldPx>) -> Rect<Tiles> {
        Rect::new(
            self.point(r.origin),
            Size::new(r.size.width * self.x.get(), r.size.height * self.y.get()),
        )
    }

    pub fn tile_center(self, p: Pos<WorldPx>) -> Pos<Tiles> {
        let t = self.point(p);
        Pos::new(t.x.floor() + 0.5, t.y.floor() + 0.5)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    S = 0,
    SW = 1,
    NW = 2,
    N = 3,
    NE = 4,
    SE = 5,
    E = 6,
    W = 7,
}

impl Direction {
    pub fn from_vec(dx: f32, dy: f32) -> Direction {
        if dx == 0.0 && dy == 0.0 {
            return Direction::S;
        }
        const D: f32 = FRAC_1_SQRT_2;
        let candidates = [
            (Direction::E, 1.0, 0.0),
            (Direction::SE, D, D),
            (Direction::S, 0.0, 1.0),
            (Direction::SW, -D, D),
            (Direction::W, -1.0, 0.0),
            (Direction::NW, -D, -D),
            (Direction::N, 0.0, -1.0),
            (Direction::NE, D, -D),
        ];
        let mut best = Direction::S;
        let mut best_dot = f32::NEG_INFINITY;
        for (dir, cx, cy) in candidates {
            let dot = dx * cx + dy * cy;
            if dot > best_dot {
                best_dot = dot;
                best = dir;
            }
        }
        best
    }
}

// The exact sequence is part of the game: it fixes every boot's npc layout, so it cannot move to a crate rng.
pub fn next_rng(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

pub fn rng_unit(state: &mut u64) -> f32 {
    (next_rng(state) >> 40) as f32 / (1u64 << 24) as f32
}
