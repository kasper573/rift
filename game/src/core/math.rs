use std::f32::consts::FRAC_1_SQRT_2;

use bevy_ecs::prelude::Resource;
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

#[derive(Resource)]
pub struct Rng(oorandom::Rand32);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(oorandom::Rand32::new(seed))
    }

    pub fn from_entropy() -> Rng {
        let mut seed = [0u8; 8];
        getrandom::getrandom(&mut seed).expect("os entropy for rng seed");
        Rng::new(u64::from_le_bytes(seed))
    }

    pub fn rand_float(&mut self) -> f32 {
        self.0.rand_float()
    }

    pub fn rand_range(&mut self, range: std::ops::Range<u32>) -> u32 {
        self.0.rand_range(range)
    }
}
