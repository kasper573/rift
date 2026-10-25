//! The shadows of clouds drifting over the ground: a sheet of cloud that tiles, lit where the sky
//! is clear and darker under a cloud, laid over the sun and carried along by the wind.

use bevy::asset::RenderAssetUsages;
use bevy::light::DirectionalLightTexture;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::core::units::Meters;

/// How much ground one tile of the sheet covers.
pub const CLOUD_TILE: Meters = Meters(260.0);
/// How fast the clouds drift for a full wind.
pub const CLOUD_SPEED: f32 = 2.5;
/// How much of the sun a cloud takes.
const CLOUD_SHADE: f32 = 0.45;
const SHEET: u32 = 256;
/// Cloud cells across a tile at the coarsest; each octave doubles.
const CELLS: u32 = 4;
const OCTAVES: u32 = 5;
/// Where, in the noise's range, cloud begins and is thick.
const CLEAR: f32 = 0.5;
const THICK: f32 = 0.68;

/// The one sheet of cloud every sun carries.
#[derive(Resource)]
pub struct CloudSheet(Handle<Image>);

impl FromWorld for CloudSheet {
    fn from_world(world: &mut World) -> CloudSheet {
        CloudSheet(world.resource_mut::<Assets<Image>>().add(sheet()))
    }
}

impl CloudSheet {
    /// The sheet laid over a sun, tiled over the ground.
    pub fn over_sun(&self) -> DirectionalLightTexture {
        DirectionalLightTexture {
            image: self.0.clone(),
            tiled: true,
        }
    }
}

/// How far the clouds have drifted on `wind` after `seconds`.
pub fn drift(wind: Vec2, seconds: f32) -> Vec2 {
    wind * CLOUD_SPEED * seconds
}

/// The sheet: how much of the sun reaches the ground, 1 under a clear sky, less under cloud,
/// drawn from a few octaves of noise that wrap, so the sheet tiles without a seam.
fn sheet() -> Image {
    let mut data = Vec::with_capacity((SHEET * SHEET * 4) as usize);
    for row in 0..SHEET {
        for column in 0..SHEET {
            let at = Vec2::new(column as f32, row as f32) / SHEET as f32;
            let cloud = smoothstep(CLEAR, THICK, noise(at));
            let lit = ((1.0 - CLOUD_SHADE * cloud) * 255.0) as u8;
            data.extend([lit, lit, lit, 255]);
        }
    }
    Image::new(
        Extent3d {
            width: SHEET,
            height: SHEET,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// Octaves of value noise over the unit square, each wrapping at its edges, in 0..1.
fn noise(at: Vec2) -> f32 {
    let mut sum = 0.0;
    let mut weight = 0.5;
    let mut total = 0.0;
    for octave in 0..OCTAVES {
        let cells = CELLS << octave;
        sum += weight * value_noise(at * cells as f32, cells, octave);
        total += weight;
        weight *= 0.5;
    }
    sum / total
}

/// Value noise on a lattice of `cells` a side, wrapping, blended between the lattice's corners.
fn value_noise(at: Vec2, cells: u32, octave: u32) -> f32 {
    let cell = at.floor();
    let t = at - cell;
    let fade = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let (fx, fy) = (fade(t.x), fade(t.y));
    let corner = |dx: u32, dy: u32| {
        let x = (cell.x as u32 + dx) % cells;
        let y = (cell.y as u32 + dy) % cells;
        hash(x, y, octave)
    };
    let low = corner(0, 0) + (corner(1, 0) - corner(0, 0)) * fx;
    let high = corner(0, 1) + (corner(1, 1) - corner(0, 1)) * fx;
    low + (high - low) * fy
}

fn hash(x: u32, y: u32, octave: u32) -> f32 {
    let mut state = x
        .wrapping_mul(374_761_393)
        .wrapping_add(y.wrapping_mul(668_265_263))
        .wrapping_add(octave.wrapping_mul(2_246_822_519));
    state = (state ^ (state >> 13)).wrapping_mul(1_274_126_177);
    (state ^ (state >> 16)) as f32 / u32::MAX as f32
}

fn smoothstep(from: f32, to: f32, at: f32) -> f32 {
    let t = ((at - from) / (to - from)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
