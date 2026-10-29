//! The clouds over an area: one field of cloud lying over the world at the clouds' height,
//! carried along by the wind and as thick as the weather's cover, which the sky paints the clouds
//! from, and laid over the sun along its rays, so the ground lies darker under every cloud
//! overhead, where the light's rays carry its shadow.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::light::DirectionalLightTexture;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::core::units::Meters;

/// How much ground one tile of the field covers, the field repeating past it.
pub const CLOUD_TILE: Meters = Meters(640.0);
/// The height the field of cloud lies at.
pub const CLOUD_BASE: Meters = Meters(150.0);
/// How much ground the field's shadows reach over round where they are laid, repeating past it.
pub const SHADOW_SPAN: Meters = Meters(512.0);
/// The lowest a light stands over the horizon, in degrees, for the field to cast its shadows.
pub const LOWEST_LIGHT: f32 = 2.0;
/// How fast the clouds drift for a full wind.
pub const CLOUD_SPEED: f32 = 2.5;
/// How much of the sun a cloud takes, from fair-weather cloud to storm cloud.
const FAIR_SHADE: f32 = 0.35;
const STORM_SHADE: f32 = 0.9;
/// Texels a side of the field and of its shadows.
const SHEET: u32 = 256;
const SHADOWS: u32 = 256;
/// How far the light and the clouds travel before the shadows are laid anew: well within the span
/// they reach over, so the ground never sees where they repeat.
const RELAID_AFTER: Meters = Meters(32.0);
/// Cloud cells across a tile at the coarsest; each octave doubles.
const CELLS: u32 = 4;
const OCTAVES: u32 = 3;
/// How far, in tiles, the field's noise is pushed about where it is read.
const WARP: f32 = 0.12;
/// Where, in the noise's range, cloud begins under a clear sky and under a full cover, and how
/// far past that it is thick.
const CLEAR_SKY: f32 = 0.8;
const FULL_COVER: f32 = 0.2;
pub const SOFTNESS: f32 = 0.16;
/// The second read of the noise every point blends with its first: turned and scaled by amounts
/// no small whole numbers relate, so the two only fall in step again far past anything in view,
/// and the turn crosses the rows the noise's square lattice would otherwise line its clouds up in.
const SECOND_TURN: f32 = 1.0;
const SECOND_SCALE: f32 = 0.754_877_7;
pub const SECOND_SHIFT: Vec2 = Vec2::new(0.373, 0.619);

/// The one field of cloud every sky paints and every sun casts: `field`, the noise the clouds are
/// drawn from in red, read twice and blended where it is read (`read`), so it never shows where it
/// repeats, and thresholded at the cover's `edge`; and `shadows`, how much of the light reaches
/// the ground through it, laid along the light's rays as the light and the drift last stood.
#[derive(Resource)]
pub struct CloudSheet {
    field: Handle<Image>,
    shadows: Handle<Image>,
    noise: Vec<f32>,
    weather: (f32, f32),
    laid: Option<Laid>,
}

impl FromWorld for CloudSheet {
    fn from_world(world: &mut World) -> CloudSheet {
        let noise: Vec<f32> = (0..SHEET * SHEET)
            .map(|texel| {
                noise(Vec2::new((texel % SHEET) as f32, (texel / SHEET) as f32) / SHEET as f32)
            })
            .collect();
        let mut images = world.resource_mut::<Assets<Image>>();
        let field = images.add(picture(
            SHEET,
            noise
                .iter()
                .flat_map(|&noise| [noise, 0.0, 0.0, 1.0].map(|value| (value * 255.0) as u8))
                .collect(),
        ));
        let shadows = images.add(picture(
            SHADOWS,
            vec![255; (SHADOWS * SHADOWS * 4) as usize],
        ));
        CloudSheet {
            field,
            shadows,
            noise,
            weather: (0.0, 0.0),
            laid: None,
        }
    }
}

impl CloudSheet {
    /// The field's shadows laid over a sun.
    pub fn over_sun(&self) -> DirectionalLightTexture {
        DirectionalLightTexture {
            image: self.shadows.clone(),
            tiled: true,
        }
    }

    /// The field as the sky reads it.
    pub fn field(&self) -> Handle<Image> {
        self.field.clone()
    }

    /// How cloudy the sky is at `at`, in tiles, under the cover last set, 0 clear to 1 thick, as
    /// the sky reads the field.
    pub fn cloudiness(&self, at: Vec2) -> f32 {
        let edge = edge(self.weather.0);
        smoothstep(edge, edge + SOFTNESS, read(&self.noise, at))
    }

    /// Thickens or thins the field's shadows to cover `clouds` of the sky, 0 none and 1 all of
    /// it, as dark as cloud holding `water` casts.
    pub fn cover(&mut self, clouds: f32, water: f32) {
        if self.weather != (clouds, water) {
            self.weather = (clouds, water);
            self.laid = None;
        }
    }

    /// Lays the field's shadows along the rays of a light standing as `light` does, the field
    /// carried `drift` along the ground, and returns where the light stands to cast them. The
    /// shadows are laid anew only once the light has turned far enough to slip them a texel, or
    /// it and the clouds have travelled `RELAID_AFTER` since; until then the light is carried
    /// along with the clouds, so the shadows last laid fall where the clouds have drifted since.
    pub fn lay(&mut self, light: Transform, drift: Vec2, images: &mut Assets<Image>) -> Transform {
        let laid = match self.laid.filter(|laid| laid.holds(light, drift)) {
            Some(laid) => laid,
            None => {
                self.paint_shadows(light, drift, images);
                Laid { light, drift }
            }
        };
        self.laid = Some(laid);
        let carried = drift - laid.drift;
        Transform {
            translation: laid.light.translation + Vec3::new(carried.x, 0.0, carried.y),
            ..light
        }
    }

    fn paint_shadows(&self, light: Transform, drift: Vec2, images: &mut Assets<Image>) {
        let shade = shade(self.weather.1);
        let toward = light.back().as_vec3();
        let data = (0..SHADOWS * SHADOWS)
            .flat_map(|texel| {
                let uv = (Vec2::new((texel % SHADOWS) as f32, (texel / SHADOWS) as f32) + 0.5)
                    / SHADOWS as f32;
                let near =
                    light.transform_point(Vec3::new(1.0 - 2.0 * uv.x, 2.0 * uv.y - 1.0, 0.0));
                let over = near + toward * (CLOUD_BASE.0 - near.y) / toward.y;
                let at = (Vec2::new(over.x, over.z) - drift) / CLOUD_TILE.0;
                let lit = 1.0 - shade * self.cloudiness(at);
                [lit, lit, lit, 1.0].map(|value| (value * 255.0) as u8)
            })
            .collect();
        if let Some(mut image) = images.get_mut(&self.shadows) {
            *image = picture(SHADOWS, data);
        }
    }
}

/// How much of the sun a cloud holding `water` takes.
pub fn shade(water: f32) -> f32 {
    FAIR_SHADE + (STORM_SHADE - FAIR_SHADE) * water.clamp(0.0, 1.0)
}

/// How far the clouds have drifted on `wind` after `seconds`.
pub fn drift(wind: Vec2, seconds: f32) -> Vec2 {
    wind * CLOUD_SPEED * seconds
}

/// Where the shadows were last laid: along the rays of `light`, the field carried `drift`.
#[derive(Clone, Copy)]
struct Laid {
    light: Transform,
    drift: Vec2,
}

impl Laid {
    /// Whether the shadows laid here still fall true under `light` with the field carried `drift`.
    fn holds(&self, light: Transform, drift: Vec2) -> bool {
        let slip = CLOUD_BASE.0 * (lean(self.light) - lean(light)).length();
        let travelled =
            self.light.translation.distance(light.translation) + self.drift.distance(drift);
        slip < SHADOW_SPAN.0 / SHADOWS as f32 && travelled < RELAID_AFTER.0
    }
}

/// How far a light's rays run along the ground for every metre they fall.
fn lean(light: Transform) -> Vec2 {
    let toward = light.back();
    Vec2::new(toward.x, toward.z) / toward.y
}

/// Where, in the noise's range, cloud begins under `clouds`.
pub fn edge(clouds: f32) -> f32 {
    CLEAR_SKY + (FULL_COVER - CLEAR_SKY) * clouds.clamp(0.0, 1.0)
}

/// How the second read of the noise is turned and scaled from the first, in tiles; it is then
/// carried `SECOND_SHIFT`.
pub fn second_read() -> Mat2 {
    Mat2::from_scale_angle(Vec2::splat(SECOND_SCALE), SECOND_TURN)
}

/// The field's noise at `at`, in tiles: its two reads blended, as varied as either alone.
fn read(noise: &[f32], at: Vec2) -> f32 {
    let first = sampled(noise, at);
    let second = sampled(noise, second_read() * at + SECOND_SHIFT);
    0.5 + (first + second - 1.0) * std::f32::consts::FRAC_1_SQRT_2
}

/// The field's noise at `at`, in tiles, blended between texels and repeating every tile.
fn sampled(cloud: &[f32], at: Vec2) -> f32 {
    let texel = at * SHEET as f32 - 0.5;
    let corner = texel.floor();
    let t = texel - corner;
    let value = |dx: i32, dy: i32| {
        let x = (corner.x as i32 + dx).rem_euclid(SHEET as i32) as usize;
        let y = (corner.y as i32 + dy).rem_euclid(SHEET as i32) as usize;
        cloud[y * SHEET as usize + x]
    };
    let low = value(0, 0) + (value(1, 0) - value(0, 0)) * t.x;
    let high = value(0, 1) + (value(1, 1) - value(0, 1)) * t.x;
    low + (high - low) * t.y
}

/// A square picture `size` texels a side, repeating every way and blended between texels.
fn picture(size: u32, data: Vec<u8>) -> Image {
    let mut image = Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..default()
    });
    image
}

/// The noise the field is drawn from over the unit square, wrapping at its edges, in 0..1: octaves
/// of value noise read at a point pushed about by two more, so the outlines it draws stretch and
/// curl as clouds do rather than ringing round each peak.
fn noise(at: Vec2) -> f32 {
    let push = Vec2::new(octaves(at, 1), octaves(at, 2)) - 0.5;
    octaves((at + push * WARP).rem_euclid(Vec2::ONE), 0)
}

/// Octaves of value noise over the unit square, each wrapping at its edges, in 0..1; each `salt`
/// draws a noise of its own.
fn octaves(at: Vec2, salt: u32) -> f32 {
    let mut sum = 0.0;
    let mut weight = 0.5;
    let mut total = 0.0;
    for octave in 0..OCTAVES {
        let cells = CELLS << octave;
        sum += weight * value_noise(at * cells as f32, cells, salt * OCTAVES + octave);
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
