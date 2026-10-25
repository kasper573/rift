//! The sky behind the world and the daylight it stands in: a cubemap shaded from the horizon's
//! colour up to the zenith's, built for the area on view, and how an area's authored sun, sky and
//! ambient light turn with the hour of a day cycle and dim under cloud.

use std::f32::consts::PI;

use bevy::asset::RenderAssetUsages;
use bevy::color::Mix;
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
};

use super::lights::SUN_LUX;
use crate::core::geometry::Degrees;

const FACE: u32 = 32;
/// How bright the sky is drawn, so its colours show as authored under the camera's exposure.
pub const SKY_NITS: f32 = 1000.0;

/// Where the game stands in its day: at a fixed hour that never moves, so an area's light stands
/// as authored, or at an hour of a cycle that turns through day and night.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub enum Daylight {
    #[default]
    Fixed,
    Hour(f32),
}

impl Daylight {
    /// The hour as a share of the day, for a shader; noon when fixed.
    pub fn share(self) -> f32 {
        match self {
            Daylight::Fixed => 0.5,
            Daylight::Hour(hour) => hour / 24.0,
        }
    }

    /// The hour in steps of a minute and a half, so lighting is redone only as the light moves.
    pub fn step(self) -> u32 {
        match self {
            Daylight::Fixed => 0,
            Daylight::Hour(hour) => (hour * 40.0) as u32,
        }
    }
}

/// A sun as it shines at some hour: where it stands and how.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SunLight {
    pub elevation: Degrees,
    pub azimuth: Degrees,
    pub color: Color,
    pub illuminance: f32,
}

/// The colours of a sky, overhead and at the horizon.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyColors {
    pub zenith: Color,
    pub horizon: Color,
}

const NIGHT: SkyColors = SkyColors {
    zenith: Color::srgb(0.07, 0.10, 0.23),
    horizon: Color::srgb(0.30, 0.26, 0.38),
};
const DUSK: SkyColors = SkyColors {
    zenith: Color::srgb(0.24, 0.27, 0.47),
    horizon: Color::srgb(0.95, 0.60, 0.36),
};
const OVERCAST: SkyColors = SkyColors {
    zenith: Color::srgb(0.58, 0.62, 0.66),
    horizon: Color::srgb(0.78, 0.80, 0.82),
};
const SETTING_SUN: Color = Color::srgb(1.0, 0.6, 0.32);
/// The sun's elevation below which its light is gone and above which it is full.
const SUNRISE: (f32, f32) = (-6.0, 12.0);
/// How much of the sun cloud takes.
const CLOUD_SHADE: f32 = 0.75;

/// An area's authored sun as it shines at `daylight` under `overcast` skies: fixed, as written;
/// at an hour, climbing from the east over the morning to its authored noon height and bearing
/// and setting over the evening, reddening as it nears the horizon and going out beneath it.
pub fn sun_at(
    daylight: Daylight,
    noon_elevation: Degrees,
    noon_azimuth: Degrees,
    color: Color,
    overcast: f32,
) -> SunLight {
    let clouded = 1.0 - CLOUD_SHADE * overcast;
    match daylight {
        Daylight::Fixed => SunLight {
            elevation: noon_elevation,
            azimuth: noon_azimuth,
            color,
            illuminance: SUN_LUX * clouded,
        },
        Daylight::Hour(hour) => {
            let arc = ((hour - 6.0) / 12.0 * PI).sin();
            let elevation = Degrees(noon_elevation.0 * arc);
            let warmth = 1.0 - smoothstep(0.0, 25.0, elevation.0);
            SunLight {
                elevation,
                azimuth: Degrees(noon_azimuth.0 + (hour - 12.0) * 15.0),
                color: mix(color, SETTING_SUN, warmth * 0.8),
                illuminance: SUN_LUX * smoothstep(SUNRISE.0, SUNRISE.1, elevation.0) * clouded,
            }
        }
    }
}

/// How much of its authored brightness an area's ambient light keeps with its sun at
/// `elevation` (none for an area without one) and under `overcast` skies.
pub fn ambient_at(daylight: Daylight, elevation: Option<Degrees>, overcast: f32) -> f32 {
    daytime(daylight, elevation).map_or(1.0, |lit| 0.3 + 0.7 * lit) * (1.0 - 0.2 * overcast)
}

/// An area's authored sky `day` as it shows with its sun at `elevation` (none for an area without
/// one) under `overcast` skies: by night the night's colours, glowing at the horizon around dawn
/// and dusk, and greyed by cloud.
pub fn sky_at(
    day: SkyColors,
    daylight: Daylight,
    elevation: Option<Degrees>,
    overcast: f32,
) -> SkyColors {
    let (zenith, horizon, lit) = match daytime(daylight, elevation) {
        None => (day.zenith, day.horizon, 1.0),
        Some(lit) => {
            let glow = (1.0 - elevation.map_or(90.0, |elevation| elevation.0.abs()) / 15.0)
                .clamp(0.0, 1.0);
            (
                mix(mix(NIGHT.zenith, day.zenith, lit), DUSK.zenith, glow * 0.4),
                mix(mix(NIGHT.horizon, day.horizon, lit), DUSK.horizon, glow),
                lit,
            )
        }
    };
    let dim = 0.3 + 0.7 * lit;
    SkyColors {
        zenith: mix(zenith, dimmed(OVERCAST.zenith, dim), overcast),
        horizon: mix(horizon, dimmed(OVERCAST.horizon, dim), overcast),
    }
}

/// How far into the day a sun at `elevation` is, 0 by night and 1 by day, or nothing when the
/// light stands fixed or there is no sun to move.
fn daytime(daylight: Daylight, elevation: Option<Degrees>) -> Option<f32> {
    match (daylight, elevation) {
        (Daylight::Hour(_), Some(elevation)) => Some(smoothstep(-10.0, 10.0, elevation.0)),
        _ => None,
    }
}

fn mix(from: Color, to: Color, by: f32) -> Color {
    Color::LinearRgba(from.to_linear().mix(&to.to_linear(), by))
}

fn dimmed(color: Color, by: f32) -> Color {
    Color::LinearRgba(color.to_linear() * by)
}

fn smoothstep(from: f32, to: f32, at: f32) -> f32 {
    let t = ((at - from) / (to - from)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A cubemap that is the sky's horizon colour at and below the horizon and shades up to its
/// zenith overhead.
pub fn cubemap(sky: SkyColors) -> Image {
    let (zenith, horizon) = (sky.zenith.to_srgba(), sky.horizon.to_srgba());
    let mut data = Vec::with_capacity((FACE * FACE * 6 * 4) as usize);
    for face in 0..6 {
        for row in 0..FACE {
            for column in 0..FACE {
                let u = (column as f32 + 0.5) / FACE as f32 * 2.0 - 1.0;
                let v = (row as f32 + 0.5) / FACE as f32 * 2.0 - 1.0;
                let direction = match face {
                    0 => Vec3::new(1.0, -v, -u),
                    1 => Vec3::new(-1.0, -v, u),
                    2 => Vec3::new(u, 1.0, v),
                    3 => Vec3::new(u, -1.0, -v),
                    4 => Vec3::new(u, -v, 1.0),
                    _ => Vec3::new(-u, -v, -1.0),
                }
                .normalize();
                let up = direction.y.clamp(0.0, 1.0).powf(0.55);
                let channel = |low: f32, high: f32| ((low + (high - low) * up) * 255.0) as u8;
                data.extend([
                    channel(horizon.red, zenith.red),
                    channel(horizon.green, zenith.green),
                    channel(horizon.blue, zenith.blue),
                    255,
                ]);
            }
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: FACE,
            height: FACE,
            depth_or_array_layers: 6,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::Cube),
        ..default()
    });
    image
}
