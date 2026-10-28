//! The sky over the world and the daylight it stands in: an atmosphere scattering the sun's light,
//! which paints the sky, blues the distance and lights everything from above, or the dark of a
//! cave or a stage; the air the sun's shafts show in, and how an area's authored sun and ambient
//! light turn with the hour of a day cycle and dim under cloud.

use std::f32::consts::PI;

use bevy::color::Mix;
use bevy::light::atmosphere::ScatteringMedium;
use bevy::light::{Atmosphere, AtmosphereEnvironmentMapLight, FogVolume};
use bevy::pbr::{AtmosphereSettings, DistanceFog, FogFalloff};
use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::lights::SUN_LUX;
use crate::core::geometry::Degrees;
use crate::core::math::Rect;
use crate::core::render::camera::FullPicture;
use crate::core::units::Meters;

/// How many metres of air a metre of the world stands for: enough that the sky's horizon shows
/// its haze band from the heights the camera stands at, and no more, so what stands near keeps
/// its own colour. The world is a diorama a few hundred metres across, and the haze that blues its
/// far side is laid on separately, clear up close.
const AIR_PER_METRE: f32 = 10.0;
/// How far from the camera the air stays clear, and how far until the haze hides everything in
/// the sky's colour.
const HAZE_CLEAR: f32 = 50.0;
const HAZE_THICK: f32 = 170.0;
const DAY_HAZE: Color = Color::srgb(0.72, 0.83, 0.96);
const DUSK_HAZE: Color = Color::srgb(0.95, 0.66, 0.42);
const NIGHT_HAZE: Color = Color::srgb(0.16, 0.18, 0.30);
/// How far into the air the haze is worked out, in metres of the world.
const HAZE_REACH: f32 = 600.0;
/// How much the sky lights everything from above, of the atmosphere's own brightness.
const SKY_LIGHT: f32 = 0.3;
/// How many pixels across each face of the map of the sky's light is: the sky is a smooth
/// gradient, and the light it sheds on a matte face smoother still.
const SKY_MAP: u32 = 32;
/// How high the air the sun's shafts show in reaches over an area.
const AIR_HEIGHT: f32 = 40.0;
/// How thick that air is: thin, so it only shows where the light breaks through a canopy or a
/// window, never as a fog over the ground.
const AIR_DENSITY: f32 = 0.025;

/// What stands over an area: the air its sun lights, blue by day, or the dark of a cave or a
/// stage, which no sun lights however it shines below.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Sky {
    #[default]
    Air,
    Dark,
}

impl Sky {
    pub fn is_air(&self) -> bool {
        *self == Sky::Air
    }
}

/// Lets `camera` see `sky` and be lit by it: the air drawn behind the world, hazing the distance
/// and shedding its light from above, where the GPU draws the full picture; nothing over the
/// dark, which takes the air of any other sky standing in the world out of the camera's view.
pub fn show(world: &mut World, camera: Entity, sky: Sky) {
    let full = world.resource::<FullPicture>().0;
    let Ok(mut camera) = world.get_entity_mut(camera) else {
        return;
    };
    if sky.is_air() && full {
        if !camera.contains::<AtmosphereEnvironmentMapLight>() {
            camera.insert((
                AtmosphereSettings {
                    aerial_view_lut_max_distance: HAZE_REACH,
                    ..default()
                },
                AtmosphereEnvironmentMapLight {
                    intensity: SKY_LIGHT,
                    size: UVec2::splat(SKY_MAP),
                    ..default()
                },
            ));
        }
    } else {
        camera.remove::<(AtmosphereSettings, AtmosphereEnvironmentMapLight)>();
    }
}

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
}

/// A sun as it shines at some hour: where it stands and how.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SunLight {
    pub elevation: Degrees,
    pub azimuth: Degrees,
    pub color: Color,
    pub illuminance: f32,
}

const SETTING_SUN: Color = Color::srgb(1.0, 0.6, 0.32);
/// The sun's elevation below which its light is gone and above which it is full.
const SUNRISE: (f32, f32) = (-6.0, 12.0);
/// How much of the sun cloud takes.
const CLOUD_SHADE: f32 = 0.75;

/// The sky over the world: an Earth's atmosphere, scaled down so the world's few hundred metres
/// stretch over enough air to haze, over a ground the colour of the haze, so that past the
/// world's edge the eye meets the same haze the far distance sinks into.
pub fn atmosphere(media: &mut Assets<ScatteringMedium>) -> impl Bundle {
    let mut atmosphere = Atmosphere::earth(media.add(ScatteringMedium::default()));
    atmosphere.ground_albedo = DAY_HAZE.to_linear().to_vec3();
    let scale = 1.0 / AIR_PER_METRE;
    (
        Transform {
            translation: -Vec3::Y * atmosphere.inner_radius * scale,
            scale: Vec3::splat(scale),
            ..default()
        },
        atmosphere,
    )
}

/// The air standing over `ground`, which the sun's light shafts show in.
pub fn air(ground: Rect<Meters>) -> impl Bundle {
    let center = ground.center();
    (
        FogVolume {
            density_factor: AIR_DENSITY,
            absorption: 0.1,
            scattering: 0.3,
            scattering_asymmetry: 0.7,
            ..default()
        },
        Transform::from_xyz(center.x, AIR_HEIGHT / 2.0, center.y).with_scale(Vec3::new(
            ground.size.width,
            AIR_HEIGHT,
            ground.size.height,
        )),
    )
}

/// The haze the camera sees the world through under a `sun` and a `sky`: clear up close,
/// thickening with the distance into the colour of the sky at the horizon, pale blue by day,
/// glowing at dusk and dark by night or under a dark sky, so what stands far off sinks into the
/// sky.
pub fn haze(sun: Option<SunLight>, sky: Sky) -> DistanceFog {
    let color = match (sky, sun) {
        (Sky::Dark, _) => Color::BLACK,
        (Sky::Air, None) => NIGHT_HAZE,
        (Sky::Air, Some(sun)) => {
            let lit = smoothstep(SUNRISE.0, SUNRISE.1, sun.elevation.0);
            let glow = (1.0 - sun.elevation.0.abs() / 15.0).clamp(0.0, 1.0);
            mix(mix(NIGHT_HAZE, DAY_HAZE, lit), DUSK_HAZE, glow)
        }
    };
    DistanceFog {
        color,
        directional_light_color: Color::NONE,
        directional_light_exponent: 8.0,
        falloff: FogFalloff::Linear {
            start: HAZE_CLEAR,
            end: HAZE_THICK,
        },
    }
}

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

fn smoothstep(from: f32, to: f32, at: f32) -> f32 {
    let t = ((at - from) / (to - from)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
