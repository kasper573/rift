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
use super::lights;
use crate::core::geometry::Degrees;
use crate::core::math::{Rect, Rgb};
use crate::core::render::Look;
use crate::core::render::camera::FullPicture;
use crate::core::units::{Meters, Stops};

/// How far into the air the haze is worked out, in metres of the world.
const HAZE_REACH: f32 = 600.0;
/// How many pixels across each face of the map of the sky's light is: the sky is a smooth
/// gradient, and the light it sheds on a matte face smoother still.
const SKY_MAP: u32 = 32;

/// How the air over the world looks, and how it and the sun light it through the day.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SkyLook {
    /// How many metres of air a metre of the world stands for: enough that the sky's horizon
    /// shows its haze band from the heights the camera stands at, and no more, so what stands
    /// near keeps its own colour.
    pub air_per_metre: f32,
    /// How far from the camera the air stays clear, how far until the haze lies as thick as it
    /// gets, and how thick that is: never so thick that what stands in it loses its shape.
    pub haze_clear: Meters,
    pub haze_thick: Meters,
    pub thickest_haze: f32,
    /// The haze's colours under the noon light: by day, glowing toward dusk, and by night, which
    /// the dark of the night dims as it dims everything else.
    pub day_haze: Rgb,
    pub dusk_haze: Rgb,
    pub night_haze: Rgb,
    /// The haze of an air no sun ever lights.
    pub sunless_haze: Rgb,
    /// The haze of saturated air by day, and how much nearer it closes in.
    pub moist_haze: Rgb,
    pub moist_closer: f32,
    /// How many times more the water in saturated air scatters than dry air's dust.
    pub moist_scatter: f32,
    /// How moist cloud of no water at all leaves the air, of saturated.
    pub moisture_of_cloud: f32,
    /// How much of the light a face under a clear sky gets comes from the sky rather than the sun.
    pub sky_share: f32,
    /// How much the sky lights everything from above, of the atmosphere's own brightness.
    pub sky_light: f32,
    /// How high the air the sun's shafts show in reaches over an area, and how thick it is dry
    /// and saturated: dry air thin, so the shafts show only where the light breaks through, and
    /// moist air thick enough that the light stands in it in shafts.
    pub shaft_height: Meters,
    pub dry_air: f32,
    pub moist_air: f32,
    /// The colour a sun reddens toward as it nears the horizon.
    pub setting_sun: Rgb,
    /// The full moon's light, and how much of the sun's it sheds.
    pub moonlight: Rgb,
    pub moon_share: f32,
    /// How many stops the eye opens for every stop the light falls, and how many at the most.
    pub adapting: f32,
    pub most_adapted: Stops,
    /// The sun's elevation above which the night has not begun to fall, and below which it is
    /// full.
    pub dusk_begins: Degrees,
    pub night_falls: Degrees,
    /// The sun's elevation below which its light is gone, and above which it is full.
    pub sun_gone: Degrees,
    pub sun_full: Degrees,
}

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
    let Some(sky_light) = world.get_resource::<Look>().map(|look| look.sky.sky_light) else {
        return;
    };
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
            Sky::Air => sky_light,
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

/// The air over the world under `sky`: an Earth's atmosphere as moist as `humidity`, the water in
/// the air scattering the light white and hazing the horizon the more it holds, scaled down so the
/// world's few hundred metres stretch over enough air to haze, over a ground the colour of the
/// haze, so that past the world's edge the eye meets the same haze the far distance sinks into;
/// or over the dark, an air that scatters nothing over a black ground.
pub fn atmosphere(
    media: &mut Assets<ScatteringMedium>,
    sky: Sky,
    humidity: f32,
    look: &SkyLook,
) -> impl Bundle {
    let mut medium = ScatteringMedium::default();
    match sky {
        Sky::Air => {
            for term in &mut medium.terms {
                if matches!(term.phase, PhaseFunction::Mie { .. }) {
                    term.scattering *= 1.0 + look.moist_scatter * humidity;
                }
            }
        }
        Sky::Dark => medium.terms.clear(),
    }
    let mut atmosphere = Atmosphere::earth(media.add(medium));
    atmosphere.ground_albedo = match sky {
        Sky::Air => look.day_haze.color().to_linear().to_vec3(),
        Sky::Dark => Vec3::ZERO,
    };
    let scale = 1.0 / look.air_per_metre;
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
pub fn air(ground: Rect<Meters>, humidity: f32, look: &SkyLook) -> impl Bundle {
    let center = ground.center();
    let height = look.shaft_height.0;
    (
        FogVolume {
            density_factor: look.dry_air
                + (look.moist_air - look.dry_air) * humidity.clamp(0.0, 1.0),
            absorption: 0.1,
            scattering: 0.3,
            scattering_asymmetry: 0.7,
            ..default()
        },
        Transform::from_xyz(center.x, height / 2.0, center.y).with_scale(Vec3::new(
            ground.size.width,
            height,
            ground.size.height,
        )),
    )
}

/// The haze the camera sees the world through under a `sun` and a `sky`: clear up close,
/// thickening with the distance into the colour of the sky at the horizon, pale blue by day,
/// glowing at dusk and blue by night, as bright as the world is `lit` of its noon light, or black
/// under a dark sky, so what stands far off sinks toward the sky.
pub fn haze(
    sun: Option<SunLight>,
    lit: f32,
    sky: Sky,
    humidity: f32,
    look: &SkyLook,
) -> DistanceFog {
    let color = match (sky, sun) {
        (Sky::Dark, _) => Color::BLACK,
        (Sky::Air, None) => look.sunless_haze.color(),
        (Sky::Air, Some(sun)) => {
            let day = smoothstep(look.sun_gone.0, look.sun_full.0, sun.elevation.0);
            let glow = (1.0 - sun.elevation.0.abs() / 15.0).clamp(0.0, 1.0);
            let clear = mix(look.day_haze.color(), look.moist_haze.color(), humidity);
            let hue = mix(
                mix(look.night_haze.color(), clear, day),
                look.dusk_haze.color(),
                glow,
            )
            .to_linear();
            Color::LinearRgba(hue * lit)
        }
    };
    let closer = 1.0 - look.moist_closer * humidity;
    DistanceFog {
        color: color.with_alpha(look.thickest_haze),
        directional_light_color: Color::NONE,
        directional_light_exponent: 8.0,
        falloff: FogFalloff::Linear {
            start: look.haze_clear.0 * closer,
            end: look.haze_thick.0 * closer,
        },
    }
}

/// How moist the air of an area is under `clouds` holding `water`, 0 dry to 1 saturated: the
/// more water the sky holds, the more the air does, and cloud of any kind means some.
pub fn humidity(clouds: f32, water: f32, look: &SkyLook) -> f32 {
    let dry = look.moisture_of_cloud;
    (clouds * (dry + (1.0 - dry) * water)).clamp(0.0, 1.0)
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
    look: &Look,
) -> SunLight {
    let clouded = clouded(clouds, water, look);
    let sun_lux = look.light.sun_lux;
    match daylight {
        Daylight::Fixed => SunLight {
            elevation: noon_elevation,
            azimuth: noon_azimuth,
            color,
            illuminance: sun_lux * clouded,
        },
        Daylight::Hour(hour) => {
            let (elevation, azimuth) = orbit(noon_elevation, noon_azimuth, hour);
            let warmth = 1.0 - smoothstep(0.0, 25.0, elevation.0);
            SunLight {
                elevation,
                azimuth,
                color: mix(color, look.sky.setting_sun.color(), warmth * 0.8),
                illuminance: sun_lux
                    * smoothstep(look.sky.sun_gone.0, look.sky.sun_full.0, elevation.0)
                    * clouded,
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
pub fn shining(sun: SunLight, clouds: f32, water: f32, look: &Look) -> SunLight {
    let (gone, full) = (look.sky.sun_gone.0, look.sky.sun_full.0);
    if sun.elevation.0 >= gone {
        return sun;
    }
    let elevation = Degrees(-sun.elevation.0);
    SunLight {
        elevation,
        azimuth: Degrees((sun.azimuth.0 + 180.0).rem_euclid(360.0)),
        color: look.sky.moonlight.color(),
        illuminance: look.light.sun_lux
            * look.sky.moon_share
            * smoothstep(-gone, full - 2.0 * gone, elevation.0)
            * clouded(clouds, water, look),
    }
}

/// How far into the night the sky is under `sun`, 0 while any daylight lingers to 1 once the
/// stars are all out; never night when the light stands fixed.
pub fn night(daylight: Daylight, sun: SunLight, look: &SkyLook) -> f32 {
    match daylight {
        Daylight::Fixed => 0.0,
        Daylight::Hour(_) => smoothstep(look.dusk_begins.0, look.night_falls.0, sun.elevation.0),
    }
}

/// How much wider the eye opens under `shining` at `daylight`, as an eye opens to the dusk and
/// the night: by `adapting` for every stop the light falls, the sky dimming too as the light stands
/// lower, up to `most_adapted`; never under a fixed light, which stands as authored.
pub fn adaptation(daylight: Daylight, shining: SunLight, look: &Look) -> Stops {
    match daylight {
        Daylight::Fixed => Stops(0.0),
        Daylight::Hour(_) => {
            let height = shining.elevation.radians().0.sin().max(0.0).sqrt();
            let strength = shining.illuminance / look.light.sun_lux * (0.3 + 0.7 * height);
            let opened = -strength.max(1e-3).log2() * look.sky.adapting;
            Stops(opened.clamp(0.0, look.sky.most_adapted.0))
        }
    }
}

/// How much of the sun's light reaches the ground through `clouds` holding `water`.
pub fn clouded(clouds: f32, water: f32, look: &Look) -> f32 {
    1.0 - clouds::shade(water, &look.clouds) * clouds
}

/// How brightly the world shows lit by `shining` and by `ambient` of its sky's full light, through
/// an eye opened `opened`, of how it shows lit by its `noon` sun and its full sky light.
pub fn lit(shining: SunLight, ambient: f32, opened: Stops, noon: SunLight, look: &Look) -> f32 {
    let share = look.sky.sky_share;
    let shown = |light: SunLight, ambient: f32| {
        let sun =
            light.illuminance / look.light.sun_lux * light.elevation.radians().0.sin().max(0.0);
        (1.0 - share) * sun + share * ambient
    };
    shown(shining, ambient) * 2f32.powf(opened.0) / shown(noon, 1.0).max(1e-3)
}

/// How much of its authored brightness an area's ambient light keeps with its sun at
/// `elevation` (none for an area without one) and under `clouds` holding `water`: the sky's light
/// dims under cloud less than the sun's does, scattered through it rather than stopped.
pub fn ambient_at(
    daylight: Daylight,
    elevation: Option<Degrees>,
    clouds: f32,
    water: f32,
    look: &Look,
) -> f32 {
    daytime(daylight, elevation).map_or(1.0, |lit| 0.12 + 0.88 * lit)
        * (1.0 - 0.45 * clouds::shade(water, &look.clouds) * clouds)
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
