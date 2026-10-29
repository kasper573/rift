//! The sky over the world and the daylight it stands in: an atmosphere scattering the sun's light,
//! which paints the sky, blues the distance and lights everything from above, as moist as the
//! weather makes the air, or the dark of a cave or a stage; the air the sun's shafts show in, and
//! how an area's authored sun and ambient light turn with the hour of a day cycle and dim under
//! cloud, the full moon lighting the night and the eye opening to the dark.

use std::f32::consts::PI;

use bevy::color::Mix;
use bevy::light::atmosphere::{PhaseFunction, ScatteringMedium};
use bevy::light::{
    Atmosphere, AtmosphereEnvironmentMapLight, FogVolume, GeneratedEnvironmentMapLight,
};
use bevy::pbr::{AtmosphereSettings, DistanceFog, FogFalloff};
use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::clouds;
use super::lights::{self, SUN_LUX};
use crate::core::geometry::Degrees;
use crate::core::math::Rect;
use crate::core::render::camera::FullPicture;
use crate::core::units::{Meters, Stops};

/// How many metres of air a metre of the world stands for: enough that the sky's horizon shows
/// its haze band from the heights the camera stands at, and no more, so what stands near keeps
/// its own colour. The world is a diorama a few hundred metres across, and the haze that blues its
/// far side is laid on separately, clear up close.
const AIR_PER_METRE: f32 = 10.0;
/// How far from the camera the air stays clear, and how far until the haze lies as thick as it
/// gets: never so thick that what stands in it loses its shape.
const HAZE_CLEAR: f32 = 50.0;
const HAZE_THICK: f32 = 170.0;
const THICKEST_HAZE: f32 = 0.85;
/// The haze's colours under the noon light: pale blue by day, glowing toward dusk, and the blue of
/// the night, which the dark of the night dims as it dims everything else.
const DAY_HAZE: Color = Color::srgb(0.72, 0.83, 0.96);
const DUSK_HAZE: Color = Color::srgb(0.95, 0.66, 0.42);
const NIGHT_HAZE: Color = Color::srgb(0.45, 0.5, 0.85);
/// The haze of an air no sun ever lights.
const SUNLESS_HAZE: Color = Color::srgb(0.16, 0.18, 0.30);
/// How much of the light a face under a clear sky gets comes from the sky rather than the sun.
const SKY_SHARE: f32 = 0.25;
/// The haze of saturated air by day, whiter than dry air's, and how much nearer it closes in.
const MOIST_HAZE: Color = Color::srgb(0.86, 0.9, 0.93);
const MOIST_CLOSER: f32 = 0.5;
/// How many times more the water in saturated air scatters than dry air's dust.
const MOIST_MIE: f32 = 12.0;
/// How moist cloud of no water at all leaves the air, of saturated.
const MOISTURE_OF_CLOUD: f32 = 0.3;
/// How far into the air the haze is worked out, in metres of the world.
const HAZE_REACH: f32 = 600.0;
/// How much the sky lights everything from above, of the atmosphere's own brightness.
const SKY_LIGHT: f32 = 0.3;
/// How many pixels across each face of the map of the sky's light is: the sky is a smooth
/// gradient, and the light it sheds on a matte face smoother still.
const SKY_MAP: u32 = 32;
/// How high the air the sun's shafts show in reaches over an area.
const AIR_HEIGHT: f32 = 40.0;
/// How thick that air is, dry and saturated: dry air thin, so the shafts only show where the light
/// breaks through a canopy and the far side of the world keeps its colour, and moist air thick
/// enough that the light stands in it in shafts.
const DRY_AIR: f32 = 0.006;
const MOIST_AIR: f32 = 0.04;

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

/// Lets `camera` see `sky` and be lit by it, where the GPU draws the full picture: the air drawn
/// behind the world, hazing the distance and shedding its light from above, or none from the dark.
/// The camera keeps the means to draw the air for as long as it lives and only their strength
/// changes from sky to sky, since making them anew fills the GPU's command memory with the work
/// of making them, which it keeps for good.
pub fn show(world: &mut World, camera: Entity, sky: Sky) {
    if !world.resource::<FullPicture>().0 {
        return;
    }
    let Ok(mut camera) = world.get_entity_mut(camera) else {
        return;
    };
    if !camera.contains::<AtmosphereEnvironmentMapLight>() {
        camera.insert((
            AtmosphereSettings {
                aerial_view_lut_max_distance: HAZE_REACH,
                ..default()
            },
            AtmosphereEnvironmentMapLight {
                size: UVec2::splat(SKY_MAP),
                ..default()
            },
        ));
    }
    if let Some(mut light) = camera.get_mut::<GeneratedEnvironmentMapLight>() {
        light.intensity = match sky {
            Sky::Air => SKY_LIGHT,
            Sky::Dark => 0.0,
        };
    }
}

/// Where the game stands in its day: at a fixed hour that never moves, so an area's light stands
/// as authored, or at an hour of a cycle that turns through day and night.
#[derive(Resource, Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, Default, PartialEq)]
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
/// The full moon's light, and how much of the sun's it sheds.
const MOONLIGHT: Color = Color::srgb(0.62, 0.72, 1.0);
const MOON_SHARE: f32 = 0.06;
/// How many stops the eye opens for every stop the light falls, and how many at the most.
const ADAPTING: f32 = 0.5;
const MOST_ADAPTED: Stops = Stops(1.0);
/// The sun's elevation above which the night has not begun to fall and below which it is full.
const DUSK: (f32, f32) = (6.0, -10.0);
/// The sun's elevation below which its light is gone and above which it is full.
const SUNRISE: (f32, f32) = (-3.0, 8.0);

/// The air over the world under `sky`: an Earth's atmosphere as moist as `humidity`, the water in
/// the air scattering the light white and hazing the horizon the more it holds, scaled down so the
/// world's few hundred metres stretch over enough air to haze, over a ground the colour of the
/// haze, so that past the world's edge the eye meets the same haze the far distance sinks into;
/// or over the dark, an air that scatters nothing over a black ground.
pub fn atmosphere(media: &mut Assets<ScatteringMedium>, sky: Sky, humidity: f32) -> impl Bundle {
    let mut medium = ScatteringMedium::default();
    match sky {
        Sky::Air => {
            for term in &mut medium.terms {
                if matches!(term.phase, PhaseFunction::Mie { .. }) {
                    term.scattering *= 1.0 + MOIST_MIE * humidity;
                }
            }
        }
        Sky::Dark => medium.terms.clear(),
    }
    let mut atmosphere = Atmosphere::earth(media.add(medium));
    atmosphere.ground_albedo = match sky {
        Sky::Air => DAY_HAZE.to_linear().to_vec3(),
        Sky::Dark => Vec3::ZERO,
    };
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

/// The air standing over `ground` as moist as `humidity`, which the sun's light shafts show in.
pub fn air(ground: Rect<Meters>, humidity: f32) -> impl Bundle {
    let center = ground.center();
    (
        FogVolume {
            density_factor: DRY_AIR + (MOIST_AIR - DRY_AIR) * humidity.clamp(0.0, 1.0),
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
/// glowing at dusk and blue by night, as bright as the world is `lit` of its noon light, or black
/// under a dark sky, so what stands far off sinks toward the sky.
pub fn haze(sun: Option<SunLight>, lit: f32, sky: Sky, humidity: f32) -> DistanceFog {
    let color = match (sky, sun) {
        (Sky::Dark, _) => Color::BLACK,
        (Sky::Air, None) => SUNLESS_HAZE,
        (Sky::Air, Some(sun)) => {
            let day = smoothstep(SUNRISE.0, SUNRISE.1, sun.elevation.0);
            let glow = (1.0 - sun.elevation.0.abs() / 15.0).clamp(0.0, 1.0);
            let clear = mix(DAY_HAZE, MOIST_HAZE, humidity);
            let hue = mix(mix(NIGHT_HAZE, clear, day), DUSK_HAZE, glow).to_linear();
            Color::LinearRgba(hue * lit)
        }
    };
    let closer = 1.0 - MOIST_CLOSER * humidity;
    DistanceFog {
        color: color.with_alpha(THICKEST_HAZE),
        directional_light_color: Color::NONE,
        directional_light_exponent: 8.0,
        falloff: FogFalloff::Linear {
            start: HAZE_CLEAR * closer,
            end: HAZE_THICK * closer,
        },
    }
}

/// How moist the air of an area is under `clouds` holding `water`, 0 dry to 1 saturated: the
/// more water the sky holds, the more the air does, and cloud of any kind means some.
pub fn humidity(clouds: f32, water: f32) -> f32 {
    (clouds * (MOISTURE_OF_CLOUD + (1.0 - MOISTURE_OF_CLOUD) * water)).clamp(0.0, 1.0)
}

/// An area's authored sun as it shines at `daylight` under `clouds` holding `water`: fixed, as
/// written;
/// at an hour, climbing from the east over the morning to its authored noon height and bearing
/// and setting over the evening, reddening as it nears the horizon and going out beneath it.
pub fn sun_at(
    daylight: Daylight,
    noon_elevation: Degrees,
    noon_azimuth: Degrees,
    color: Color,
    clouds: f32,
    water: f32,
) -> SunLight {
    let clouded = clouded(clouds, water);
    match daylight {
        Daylight::Fixed => SunLight {
            elevation: noon_elevation,
            azimuth: noon_azimuth,
            color,
            illuminance: SUN_LUX * clouded,
        },
        Daylight::Hour(hour) => {
            let (elevation, azimuth) = orbit(noon_elevation, noon_azimuth, hour);
            let warmth = 1.0 - smoothstep(0.0, 25.0, elevation.0);
            SunLight {
                elevation,
                azimuth,
                color: mix(color, SETTING_SUN, warmth * 0.8),
                illuminance: SUN_LUX * smoothstep(SUNRISE.0, SUNRISE.1, elevation.0) * clouded,
            }
        }
    }
}

/// Where a sun standing `noon_elevation` high at `noon_azimuth` at noon stands at `hour`: on the
/// great circle through its noon place that rises due a quarter turn before its noon bearing and
/// sets a quarter turn after it, turned a twenty-fourth of the way round it each hour, as a sky
/// turns round the world.
fn orbit(noon_elevation: Degrees, noon_azimuth: Degrees, hour: f32) -> (Degrees, Degrees) {
    let noon = lights::toward(noon_elevation, noon_azimuth);
    let setting = lights::toward(Degrees(0.0), Degrees(noon_azimuth.0 + 90.0));
    let (along, across) = ((hour - 12.0) / 12.0 * PI).sin_cos();
    let way = noon * across + setting * along;
    let elevation = way.y.clamp(-1.0, 1.0).asin();
    let azimuth = way.x.atan2(-way.z);
    (
        Degrees(elevation.to_degrees()),
        Degrees(azimuth.to_degrees().rem_euclid(360.0)),
    )
}

/// The light the world is lit by under `sun`, with `clouds` holding `water`: the sun's until its
/// light is gone below the horizon, then the light of the moon, always full, standing opposite
/// it, brightening from nothing as the sun sinks on, so the one hands over to the other in the
/// dark.
pub fn shining(sun: SunLight, clouds: f32, water: f32) -> SunLight {
    if sun.elevation.0 >= SUNRISE.0 {
        return sun;
    }
    let elevation = Degrees(-sun.elevation.0);
    SunLight {
        elevation,
        azimuth: Degrees((sun.azimuth.0 + 180.0).rem_euclid(360.0)),
        color: MOONLIGHT,
        illuminance: SUN_LUX
            * MOON_SHARE
            * smoothstep(-SUNRISE.0, SUNRISE.1 - 2.0 * SUNRISE.0, elevation.0)
            * clouded(clouds, water),
    }
}

/// How far into the night the sky is under `sun`, 0 while any daylight lingers to 1 once the
/// stars are all out; never night when the light stands fixed.
pub fn night(daylight: Daylight, sun: SunLight) -> f32 {
    match daylight {
        Daylight::Fixed => 0.0,
        Daylight::Hour(_) => smoothstep(DUSK.0, DUSK.1, sun.elevation.0),
    }
}

/// How much wider the eye opens under `shining` at `daylight`, as an eye opens to the dusk and
/// the night: half a stop for every stop the light falls, the sky dimming too as the light stands
/// lower, up to one; never under a fixed light, which stands as authored.
pub fn adaptation(daylight: Daylight, shining: SunLight) -> Stops {
    match daylight {
        Daylight::Fixed => Stops(0.0),
        Daylight::Hour(_) => {
            let height = shining.elevation.radians().0.sin().max(0.0).sqrt();
            let strength = shining.illuminance / SUN_LUX * (0.3 + 0.7 * height);
            Stops((-strength.max(1e-3).log2() * ADAPTING).clamp(0.0, MOST_ADAPTED.0))
        }
    }
}

/// How much of the sun's light reaches the ground through `clouds` holding `water`.
pub fn clouded(clouds: f32, water: f32) -> f32 {
    1.0 - clouds::shade(water) * clouds
}

/// How brightly the world shows lit by `shining` and by `ambient` of its sky's full light, through
/// an eye opened `opened`, of how it shows lit by its `noon` sun and its full sky light.
pub fn lit(shining: SunLight, ambient: f32, opened: Stops, noon: SunLight) -> f32 {
    let shown = |light: SunLight, ambient: f32| {
        let sun = light.illuminance / SUN_LUX * light.elevation.radians().0.sin().max(0.0);
        (1.0 - SKY_SHARE) * sun + SKY_SHARE * ambient
    };
    shown(shining, ambient) * 2f32.powf(opened.0) / shown(noon, 1.0).max(1e-3)
}

/// How much of its authored brightness an area's ambient light keeps with its sun at
/// `elevation` (none for an area without one) and under `clouds` holding `water`: the sky's light
/// dims under cloud less than the sun's does, scattered through it rather than stopped.
pub fn ambient_at(daylight: Daylight, elevation: Option<Degrees>, clouds: f32, water: f32) -> f32 {
    daytime(daylight, elevation).map_or(1.0, |lit| 0.12 + 0.88 * lit)
        * (1.0 - 0.45 * clouds::shade(water) * clouds)
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
