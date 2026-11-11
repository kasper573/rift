//! The clouds over an area: one field of cloud lying over the world at the clouds' height,
//! carried along by the wind and as thick as the weather's cover, which the sky paints the clouds
//! from, and laid over the sun along its rays, so the ground lies darker under every cloud
//! overhead, where the light's rays carry its shadow. Some clouds hold water: which, a second noise
//! of the field says, read where the cloud lies, and the more water the weather's clouds hold, the
//! more of them do; the darker their shadows, and the rain falls from them.

use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::light::DirectionalLightTexture;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::tasks::{AsyncComputeTaskPool, Task, TaskPool};
use rayon::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::jobs::Handoff;
use crate::core::math::smoothstep;
use crate::core::units::{Meters, MetersPerSec};

/// How much ground one tile of the field covers, the field repeating past it.
pub const CLOUD_TILE: Meters = Meters(640.0);
/// How much ground the field's shadows reach over round where they are laid, repeating past it.
/// Toward their rim they even out, so each repeat joins the next.
pub const SHADOW_SPAN: Meters = Meters(512.0);
/// The lowest a light stands over the horizon, in degrees, for the field to cast its shadows.
pub const LOWEST_LIGHT: f32 = 2.0;
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
/// The second read of the noise every point blends with its first: turned and scaled by amounts
/// no small whole numbers relate, so the two only fall in step again far past anything in view,
/// and the turn crosses the rows the noise's square lattice would otherwise line its clouds up in.
const SECOND_TURN: f32 = 1.0;
const SECOND_SCALE: f32 = 0.754_877_7;
pub const SECOND_SHIFT: Vec2 = Vec2::new(0.373, 0.619);

/// How the clouds look: where they lie, how they drift, how much of the sun they take and how
/// they thicken with the weather.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CloudLook {
    /// The height the field of cloud lies at.
    pub height: Meters,
    /// How fast the clouds drift for a full wind.
    pub speed: MetersPerSec,
    /// How much of the sun a cloud takes, from fair-weather cloud to storm cloud.
    pub fair_shade: f32,
    pub storm_shade: f32,
    /// Where, in the field's noise, cloud begins under a clear sky and under a full cover, and how
    /// far past that it is thick.
    pub clear_sky: f32,
    pub full_cover: f32,
    pub softness: f32,
}

/// The one field of cloud every sky paints and every sun casts: `field`, the noise the clouds are
/// drawn from in red and the noise saying which hold water in green, each read twice and blended
/// where it is read (`read`), so it never shows where it repeats, and thresholded at the cover's
/// `edge` and the water's `water_edge`; and `shadows`, how much of the light reaches the ground
/// through it, laid along the light's rays as the light and the drift last stood. The shadows are
/// painted off the main thread, and shown the frame after they were asked for.
#[derive(Resource)]
pub struct CloudSheet {
    field: Handle<Image>,
    shadows: Handle<Image>,
    reading: CloudReading,
    laid: Option<Laid>,
    stale: bool,
    painting: Option<(Task<Vec<u8>>, Laid)>,
}

/// The field of cloud as it is read: its two noises, the weather it is thickened to and how it
/// looks. Cheap to copy, so a job off the main thread reads it as the sheet does.
#[derive(Clone)]
pub struct CloudReading {
    noise: Arc<[f32]>,
    wet: Arc<[f32]>,
    weather: (f32, f32),
    look: CloudLook,
}

impl CloudSheet {
    pub fn new(world: &mut World, look: CloudLook) -> CloudSheet {
        let texel_at =
            |texel: u32| Vec2::new((texel % SHEET) as f32, (texel / SHEET) as f32) / SHEET as f32;
        let noise: Vec<f32> = (0..SHEET * SHEET)
            .into_par_iter()
            .map(|texel| noise(texel_at(texel)))
            .collect();
        let wet: Vec<f32> = (0..SHEET * SHEET)
            .into_par_iter()
            .map(|texel| wet_noise(texel_at(texel)))
            .collect();
        let mut images = world.resource_mut::<Assets<Image>>();
        let field = images.add(picture(
            SHEET,
            noise
                .iter()
                .zip(&wet)
                .flat_map(|(&noise, &wet)| {
                    [noise, wet, 0.0, 1.0].map(|value| (value * 255.0) as u8)
                })
                .collect(),
        ));
        let shadows = images.add(picture(
            SHADOWS,
            vec![255; (SHADOWS * SHADOWS * 4) as usize],
        ));
        CloudSheet {
            field,
            shadows,
            reading: CloudReading {
                noise: noise.into(),
                wet: wet.into(),
                weather: (0.0, 0.0),
                look,
            },
            laid: None,
            stale: true,
            painting: None,
        }
    }

    /// The clouds looking as `look` says from now on.
    pub fn restyle(&mut self, look: CloudLook) {
        if self.reading.look != look {
            self.reading.look = look;
            self.stale = true;
        }
    }

    pub fn look(&self) -> &CloudLook {
        &self.reading.look
    }

    /// The field as it reads now, for a job to read away from the sheet.
    pub fn reading(&self) -> CloudReading {
        self.reading.clone()
    }
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

    /// Thickens or thins the field's shadows to cover `clouds` of the sky, 0 none and 1 all of
    /// it, with as much of it holding water as `water`, 0 none to 1 most of it.
    pub fn cover(&mut self, clouds: f32, water: f32) {
        if self.reading.weather != (clouds, water) {
            self.reading.weather = (clouds, water);
            self.stale = true;
        }
    }

    /// Lays the field's shadows along the rays of a light standing as `light` does, the field
    /// carried `drift` along the ground, and returns where the light stands to cast the shadows
    /// shown. The shadows are laid anew only once the light has turned far enough to slip them a
    /// texel, or it and the clouds have travelled `RELAID_AFTER` since, or the clouds changed;
    /// they are painted off the main thread and shown from the next lay on. Until then the light
    /// is carried along with the clouds, so the shadows last laid fall where the clouds have
    /// drifted since.
    pub fn lay(
        &mut self,
        light: Transform,
        drift: Vec2,
        images: &mut Assets<Image>,
        handoff: Handoff,
    ) -> Transform {
        if let Some((painting, laid)) = self.painting.take() {
            match handoff.take(painting) {
                Ok(data) => {
                    if let Some(mut image) = images.get_mut(&self.shadows) {
                        *image = picture(SHADOWS, data);
                    }
                    self.laid = Some(laid);
                }
                Err(painting) => self.painting = Some((painting, laid)),
            }
        }
        let height = self.reading.look.height;
        if self.painting.is_none()
            && (self.stale
                || !self
                    .laid
                    .is_some_and(|laid| laid.holds(light, drift, height)))
        {
            self.stale = false;
            let reading = self.reading.clone();
            let painting = AsyncComputeTaskPool::get_or_init(TaskPool::default)
                .spawn(async move { reading.shadows(light, drift) });
            self.painting = Some((painting, Laid { light, drift }));
        }
        let Some(laid) = self.laid else {
            return light;
        };
        let carried = drift - laid.drift;
        Transform {
            translation: laid.light.translation + Vec3::new(carried.x, 0.0, carried.y),
            ..light
        }
    }
}

impl CloudReading {
    /// How cloudy the sky is at `at`, in tiles, 0 clear to 1 thick, as the sky reads the field.
    pub fn cloudiness(&self, at: Vec2) -> f32 {
        let edge = edge(self.weather.0, &self.look);
        smoothstep(edge, edge + self.look.softness, read(&self.noise, at))
    }

    /// How much water the cloud at `at`, in tiles, holds, 0 none to 1 as much as a storm cloud:
    /// none where there is no cloud.
    pub fn water(&self, at: Vec2) -> f32 {
        let edge = water_edge(self.weather.1);
        let held = smoothstep(edge, edge + self.look.softness, read(&self.wet, at));
        self.cloudiness(at) * held
    }

    /// The height the field lies at.
    pub fn height(&self) -> Meters {
        self.look.height
    }

    /// The shadows' texels, laid along the rays of `light` with the field carried `drift`. Toward
    /// the rim they even out to the light all of them let through together, over the ground that
    /// passes from one repeat to the next when they are laid anew and as much again before it: so
    /// the repeats join, and no ground shows the shadows changing as they are laid.
    fn shadows(&self, light: Transform, drift: Vec2) -> Vec<u8> {
        let toward = light.back().as_vec3();
        let uv_of = |texel: u32| {
            (Vec2::new((texel % SHADOWS) as f32, (texel / SHADOWS) as f32) + 0.5) / SHADOWS as f32
        };
        let lit: Vec<f32> = (0..SHADOWS * SHADOWS)
            .into_par_iter()
            .map(|texel| {
                let uv = uv_of(texel);
                let near =
                    light.transform_point(Vec3::new(1.0 - 2.0 * uv.x, 2.0 * uv.y - 1.0, 0.0));
                let over = near + toward * (self.look.height.0 - near.y) / toward.y;
                let at = (Vec2::new(over.x, over.z) - drift) / CLOUD_TILE.0;
                let cloudy = self.cloudiness(at);
                let held = if cloudy > 0.0 {
                    self.water(at) / cloudy
                } else {
                    0.0
                };
                1.0 - shade(held, &self.look) * cloudy
            })
            .collect();
        let even = lit.iter().sum::<f32>() / lit.len() as f32;
        let relaid = RELAID_AFTER.0 / (SHADOW_SPAN.0 / 2.0);
        (0..SHADOWS * SHADOWS)
            .zip(lit)
            .flat_map(|(texel, lit)| {
                let out = (uv_of(texel) * 2.0 - 1.0).abs().max_element();
                let evened = smoothstep(1.0 - 2.0 * relaid, 1.0 - relaid, out);
                let lit = lit + (even - lit) * evened;
                [lit, lit, lit, 1.0].map(|value| (value * 255.0) as u8)
            })
            .collect()
    }
}

/// How much of the sun a cloud holding `water`, 0 none to 1 as much as a storm cloud, takes.
pub fn shade(water: f32, look: &CloudLook) -> f32 {
    look.fair_shade + (look.storm_shade - look.fair_shade) * water.clamp(0.0, 1.0)
}

/// How far the clouds have drifted along the ground, on whatever wind has blown, since the client
/// started.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct CloudDrift {
    drift: Vec2,
    at: Option<f32>,
}

impl CloudDrift {
    /// Clouds that have already drifted for `seconds` on whatever wind first blows them.
    pub fn carried_for(seconds: f32) -> CloudDrift {
        CloudDrift {
            drift: Vec2::ZERO,
            at: Some(-seconds),
        }
    }

    /// The drift at `seconds` into the client's run, the clouds carried on `wind` since it was
    /// last asked.
    pub fn blown(&mut self, wind: Vec2, seconds: f32, look: &CloudLook) -> Vec2 {
        let since = self.at.map_or(0.0, |at| (seconds - at).max(0.0));
        self.drift += wind * look.speed.0.0 * since;
        self.at = Some(seconds);
        self.drift
    }

    /// The drift as it was last blown.
    pub fn drift(&self) -> Vec2 {
        self.drift
    }
}

/// Where the shadows were last laid: along the rays of `light`, the field carried `drift`.
#[derive(Clone, Copy)]
struct Laid {
    light: Transform,
    drift: Vec2,
}

impl Laid {
    /// Whether the shadows laid here still fall true under `light` with the field carried `drift`.
    fn holds(&self, light: Transform, drift: Vec2, height: Meters) -> bool {
        let slip = height.0 * (lean(self.light) - lean(light)).length();
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
pub fn edge(clouds: f32, look: &CloudLook) -> f32 {
    look.clear_sky + (look.full_cover - look.clear_sky) * clouds.clamp(0.0, 1.0)
}

/// Where, in the field's second noise, cloud begins holding water when the weather's clouds hold
/// `water`: past its top for none, so no cloud does, and low in it for a storm, so most do.
pub fn water_edge(water: f32) -> f32 {
    1.0 - 0.75 * water.clamp(0.0, 1.0)
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

/// The noise saying which clouds hold water over the unit square, wrapping at its edges, in 0..1:
/// broad and slow beside the clouds' own, so a cloud holds water through and through, and drawn
/// apart from it, so it is the weather that says which clouds do, not how thick they are.
fn wet_noise(at: Vec2) -> f32 {
    octaves(at, 3)
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
