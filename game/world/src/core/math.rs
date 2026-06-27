use std::f32::consts::FRAC_1_SQRT_2;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct WorldPx(pub f32);

pub type Pos<U> = euclid::Point2D<f32, U>;
pub type Offset<U> = euclid::Vector2D<f32, U>;
pub type Size<U> = euclid::Size2D<f32, U>;
pub type Rect<U> = euclid::Rect<f32, U>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

impl<U> From<Offset<U>> for Direction {
    fn from(v: Offset<U>) -> Direction {
        if v.x == 0.0 && v.y == 0.0 {
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
            let dot = v.x * cx + v.y * cy;
            if dot > best_dot {
                best_dot = dot;
                best = dir;
            }
        }
        best
    }
}

#[derive(Clone, Copy)]
pub struct Rng(pub u64);

impl Rng {
    pub fn roll(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn unit(&mut self) -> f32 {
        (self.roll() >> 40) as f32 / (1u64 << 24) as f32
    }
}
