use std::f32::consts::FRAC_1_SQRT_2;

use bevy_ecs::prelude::Resource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, EnumString, VariantArray};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct WorldPx(pub f32);

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct WindowPx(pub f32);

#[derive(
    JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, PartialOrd, Default,
)]
#[schemars(extend("x-unit" = "%"))]
pub struct Percent(pub f32);

impl Percent {
    pub fn plus(self, other: Percent) -> Percent {
        Percent(self.0 + other.0)
    }

    pub fn times(self, count: u32) -> Percent {
        Percent(self.0 * count as f32)
    }

    pub fn rolled(self, rng: &mut Rng) -> bool {
        rng.rand_float() * 100.0 < self.0
    }
}

pub type Pos<U> = euclid::Point2D<f32, U>;
pub type Offset<U> = euclid::Vector2D<f32, U>;
pub type Size<U> = euclid::Size2D<f32, U>;
pub type Rect<U> = euclid::Rect<f32, U>;

#[derive(
    JsonSchema,
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Default,
    Serialize,
    Deserialize,
    VariantArray,
    EnumString,
    AsRefStr,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum Direction {
    #[default]
    S,
    SW,
    NW,
    N,
    NE,
    SE,
    E,
    W,
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
        Rng::new(Rng::random_seed())
    }

    // Worlds draw apart from one another, each the same way on every run with the same seed.
    pub fn for_world(seed: u64, ordinal: u64) -> Rng {
        Rng::new(seed.wrapping_add(ordinal))
    }

    pub fn random_seed() -> u64 {
        let mut seed = [0u8; 8];
        getrandom::fill(&mut seed).expect("os entropy for rng seed");
        u64::from_le_bytes(seed)
    }

    pub fn rand_float(&mut self) -> f32 {
        self.0.rand_float()
    }

    pub fn choose<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        (!items.is_empty()).then(|| &items[self.rand_range(0..items.len() as u32) as usize])
    }

    pub fn rand_range(&mut self, range: std::ops::Range<u32>) -> u32 {
        self.0.rand_range(range)
    }
}

#[derive(
    JsonSchema,
    Serialize,
    Deserialize,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Debug,
    Default,
)]
#[serde(from = "[u8; 4]", into = "[u8; 4]")]
#[schemars(extend("x-color" = true))]
pub struct Rgba(pub u32);

impl Rgba {
    pub const WHITE: Rgba = Rgba(0xffff_ffff);

    pub fn color(self) -> bevy::color::Color {
        let [r, g, b, a] = self.0.to_be_bytes();
        bevy::color::Color::srgba_u8(r, g, b, a)
    }
}

impl From<[u8; 4]> for Rgba {
    fn from(bytes: [u8; 4]) -> Rgba {
        Rgba(u32::from_be_bytes(bytes))
    }
}

impl From<Rgba> for [u8; 4] {
    fn from(rgba: Rgba) -> [u8; 4] {
        rgba.0.to_be_bytes()
    }
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameSize {
    pub width: u32,
    pub height: u32,
}
