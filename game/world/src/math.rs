//! Euclid geometry with unit tags to prevent type confusion.

use std::f32::consts::FRAC_1_SQRT_2;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct WorldPx(pub f32);

pub type Pos<U> = euclid::Point2D<f32, U>;
pub type Offset<U> = euclid::Vector2D<f32, U>;
pub type Size<U> = euclid::Size2D<f32, U>;
pub type Rect<U> = euclid::Rect<f32, U>;

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
