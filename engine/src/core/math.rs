use std::ops::Range;
use ui::egui;

use bevy_ecs::prelude::Resource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
        getrandom::u64().expect("os entropy for rng seed")
    }

    pub fn rand_float(&mut self) -> f32 {
        self.0.rand_float()
    }

    pub fn choose<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        (!items.is_empty()).then(|| &items[self.rand_range(0..items.len() as u32) as usize])
    }

    pub fn rand_range(&mut self, range: Range<u32>) -> u32 {
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

impl From<Rgba> for egui::Color32 {
    fn from(rgba: Rgba) -> egui::Color32 {
        let [r, g, b, a] = rgba.0.to_be_bytes();
        egui::Color32::from_rgba_unmultiplied(r, g, b, a)
    }
}

/// A colour with no alpha, written `[r, g, b]`: lights, the sun and a grade are never translucent.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[serde(transparent)]
#[schemars(extend("x-color" = true))]
pub struct Rgb(pub [u8; 3]);

impl Rgb {
    pub const WHITE: Rgb = Rgb([255, 255, 255]);

    pub fn color(self) -> bevy::color::Color {
        let [r, g, b] = self.0;
        bevy::color::Color::srgb_u8(r, g, b)
    }
}

impl From<Rgb> for egui::Color32 {
    fn from(rgb: Rgb) -> egui::Color32 {
        let [r, g, b] = rgb.0;
        egui::Color32::from_rgb(r, g, b)
    }
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameSize {
    #[schemars(extend("minimum" = 1))]
    pub width: u32,
    #[schemars(extend("minimum" = 1))]
    pub height: u32,
}

// The shapes the math types take in JSON, for the schemas of the types holding them: euclid's and
// glam's types know no JSON Schema of their own.

/// A point as it is written: `[x, y]`.
#[derive(JsonSchema)]
#[schemars(rename = "Pos")]
pub struct PosSchema(pub [f32; 2]);

/// An offset as it is written: `[x, y]`.
#[derive(JsonSchema)]
#[schemars(rename = "Offset")]
pub struct OffsetSchema(pub [f32; 2]);

/// A size as it is written: `[width, height]`.
#[derive(JsonSchema)]
#[schemars(rename = "Size")]
pub struct SizeSchema(pub [f32; 2]);

/// A rect as it is written: its corner nearest the origin, and its size.
#[derive(JsonSchema)]
#[schemars(rename = "Rect")]
pub struct RectSchema {
    pub origin: PosSchema,
    pub size: SizeSchema,
}

/// A 4x4 matrix as it is written: its columns one after another.
#[derive(JsonSchema)]
#[schemars(rename = "Matrix")]
pub struct MatrixSchema(pub [f32; 16]);
