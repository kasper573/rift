//! The sky over the world and the daylight it stands in: an atmosphere scattering the sun's light,
//! which paints the sky, blues the distance and lights everything from above, as moist as the
//! weather makes the air, or the dark of a cave or a stage; the air the sun's shafts show in, and
//! how an area's authored sun and ambient light turn with the hour of a day cycle and dim under
//! cloud, the full moon lighting the night and the eye opening to the dark.

use std::f32::consts::PI;

use bevy::color::Mix;
use bevy::light::atmosphere::{PhaseFunction, ScatteringMedium};
use bevy::light::{
    Atmosphere, AtmosphereEnvironmentMapLight, EnvironmentMapLight, FogVolume,
    GeneratedEnvironmentMapLight,
};
use bevy::pbr::AtmosphereSettings;
use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::haze::HazeBox;
use super::{DRAWN_FRAMES_AFTER, RenderProgress, clouds, lights};
use crate::core::math::{Rect, Rgb, smoothstep};
use crate::core::render::Look;
use crate::core::units::{Degrees, Meters, Stops};

/// How far into the air the haze is worked out, in metres of the world.
const HAZE_REACH: f32 = 600.0;
/// How many points of each slice of that reach the haze is summed over. The slices are a few
/// metres deep, across which the air is all but the same, and each ray sums its points one after
/// the other, so the GPU's time on the haze is the points' count however few the rays.
const HAZE_SAMPLES: u32 = 2;
/// How many pixels across each face of the map of the sky's light is: the sky is a smooth
/// gradient, and the light it sheds on a matte face smoother still.
const SKY_MAP: u32 = 32;
/// How far a sun turns, how much of its strength it gains or loses, and how many metres of air
/// the eye climbs, the air thinning by half a percent over them where it thins the fastest,
/// before the sky's light is taken to have changed while they are still changing.
const SUN_TURNED: Degrees = Degrees(0.1);
const SUN_BRIGHTENED: f32 = 0.005;
const AIR_CLIMBED: f32 = 6.0;
/// How many frames a camera's sky light is worked out for before it may rest: the frames its
/// pictures take to reach the GPU and be drawn into, once nothing is compiling.
const SKY_LIGHT_WARMED: u8 = DRAWN_FRAMES_AFTER as u8 + 2;
/// How many frames what sheds the sky's light must have held still before a light off by little
/// is worked out again: an eye on the move climbs in the small steps its height is told in, and
/// holds each for a few frames without having come to rest.
const SKY_RESTED: u8 = 30;

/// How the air over the world looks, and how it and the sun light it through the day.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SkyLook {
    /// How many metres of air a metre of the world stands for: enough that the sky's horizon
    /// shows its haze band from the heights the camera stands at, and no more, so what stands
    /// near keeps its own colour.
    pub air_per_metre: f32,
    /// The haze's colours under the noon light: by day, glowing toward dusk, and by night, which
    /// the dark of the night dims as it dims everything else.
    pub day_haze: Rgb,
    pub dusk_haze: Rgb,
    pub night_haze: Rgb,
    /// The haze of an air no sun ever lights.
    pub sunless_haze: Rgb,
    /// The haze of saturated air by day, and the haze of falling rain, which the air turns toward
    /// as it rains round the eye.
    pub moist_haze: Rgb,
    pub rain_haze: Rgb,
    /// How moist cloud of no water at all leaves the air, of saturated.
    pub moisture_of_cloud: f32,
    /// How much of the light a face under a clear sky gets comes from the sky rather than the sun.
    pub sky_share: f32,
    /// How much the sky lights everything from above, of the atmosphere's own brightness.
    pub sky_light: f32,
    /// The colour a sun reddens toward as it nears the horizon, the elevation below which it
    /// begins to, and how far toward it a sun on the horizon has gone, from 0 to 1.
    pub setting_sun: Rgb,
    pub reddens_below: Degrees,
    pub reddening: f32,
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
    /// How much of an area's ambient light the night leaves it, and how much of it the thickest
    /// cloud takes, each from 0 to 1.
    pub night_ambient: f32,
    pub cloud_dimming: f32,
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

/// Lets `camera` see `sky` and be lit by it: the air drawn
/// behind the world, hazing the distance and shedding its light from above, or none from the dark.
/// The camera keeps the means to draw the air for as long as it lives and only their strength
/// changes from sky to sky, since making them anew fills the GPU's command memory with the work
/// of making them, which it keeps for good.
pub fn show(world: &mut World, camera: Entity, sky: Sky) {
    let Some(sky_light) = world.get_resource::<Look>().map(|look| look.sky.sky_light) else {
        return;
    };
    let intensity = match sky {
        Sky::Air => sky_light,
        Sky::Dark => 0.0,
    };
    let Ok(mut camera) = world.get_entity_mut(camera) else {
        return;
    };
    if !camera.contains::<AtmosphereEnvironmentMapLight>() {
        camera.insert((
            AtmosphereSettings {
                aerial_view_lut_max_distance: HAZE_REACH,
                aerial_view_lut_samples: HAZE_SAMPLES,
                ..default()
            },
            AtmosphereEnvironmentMapLight {
                size: UVec2::splat(SKY_MAP),
                intensity,
                ..default()
            },
            SkyLight::default(),
        ));
    }
    if let Some(mut light) = camera.get_mut::<EnvironmentMapLight>()
        && light.intensity != intensity
    {
        light.intensity = intensity;
    }
}

/// Works the sky's light on each camera out anew only when what sheds it has changed. The light
/// is the sky's picture filtered, the same work every frame for the same light while the suns
/// and the airs stay as they are, and the filter reads the picture the frame before drew: so a
/// frame filters when the sky it would read differs from the one last filtered, at once when it
/// differs by much, and once it has come to rest when by little, so a sky at rest sheds exactly
/// its own light.
pub(super) fn rest_sky_light(
    mut commands: Commands,
    mut cameras: Query<SkyLit, With<EnvironmentMapLight>>,
    suns: Query<(&DirectionalLight, &GlobalTransform)>,
    airs: Query<(Entity, Ref<Atmosphere>, Ref<GlobalTransform>)>,
    mut media: MessageReader<AssetEvent<ScatteringMedium>>,
    progress: Option<Res<RenderProgress>>,
) {
    let Some(progress) = progress else {
        return;
    };
    let drawing = progress.drawn() > 0 && progress.compiling() == 0;
    let remade = media.read().count() > 0
        || airs
            .iter()
            .any(|(_, air, placed)| air.is_changed() || placed.is_changed());
    for (camera, mut light, eye, settings, filtering) in &mut cameras {
        light.warmth = match drawing {
            true => light.warmth.saturating_add(1),
            false if light.warmth < SKY_LIGHT_WARMED => 0,
            false => light.warmth,
        };
        let now = Shedding::seen_from(eye.translation(), &suns, &airs);
        light.rested = if light.seen.as_ref() == Some(&now) {
            light.rested.saturating_add(1)
        } else {
            0
        };
        let stale = light.warmth < SKY_LIGHT_WARMED
            || match (&light.seen, &light.filtered) {
                (Some(seen), Some(filtered)) => {
                    seen.far_from(filtered) || (seen != filtered && light.rested >= SKY_RESTED)
                }
                (seen, _) => seen.is_some(),
            };
        let seen = light.seen.replace(now);
        if stale {
            light.filtered = seen;
        }
        if remade || settings.is_changed() {
            light.filtered = None;
        }
        match (stale, filtering) {
            (true, None) => {
                if let Some(source) = light.source.clone() {
                    commands.entity(camera).insert(source);
                }
            }
            (false, Some(source)) => {
                light.source.get_or_insert_with(|| source.clone());
                commands
                    .entity(camera)
                    .remove::<GeneratedEnvironmentMapLight>();
            }
            _ => {}
        }
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

/// The air over the world under `sky`: an Earth's atmosphere whose haze scatters the light
/// `scattering` times as much as dry air's dust, the water in moist air scattering it white and
/// hazing the horizon the more it holds, scaled down so the
/// world's few hundred metres stretch over enough air to haze, over a ground the colour of the
/// haze, so that past the world's edge the eye meets the same haze the far distance sinks into;
/// or over the dark, an air that scatters nothing over a black ground.
pub fn atmosphere(
    media: &mut Assets<ScatteringMedium>,
    sky: Sky,
    scattering: f32,
    look: &SkyLook,
) -> impl Bundle {
    let mut medium = ScatteringMedium::default();
    match sky {
        Sky::Air => {
            for term in &mut medium.terms {
                if matches!(term.phase, PhaseFunction::Mie { .. }) {
                    term.scattering *= scattering;
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

/// The low air the sun's light shafts show in, as thick as `density`, standing `height` deep over
/// the ground `ground` high, over `within`. The shafts' air lets the sun's light in but no light of
/// the sky's, so a long way through it would only darken what lies behind: it stands only round
/// the eye, where the haze is still clear.
pub fn air(
    within: Rect<Meters>,
    ground: Meters,
    height: Meters,
    density: f32,
) -> (FogVolume, Transform) {
    let center = within.center();
    (
        FogVolume {
            density_factor: density,
            absorption: 0.1,
            scattering: 0.3,
            scattering_asymmetry: 0.7,
            ..default()
        },
        Transform::from_xyz(center.x, ground.0 + height.0 / 2.0, center.y).with_scale(Vec3::new(
            within.size.width.max(f32::EPSILON),
            height.0.max(f32::EPSILON),
            within.size.height.max(f32::EPSILON),
        )),
    )
}

/// How thick fog hazes what is seen through it, dry and moist: through how much of it what is
/// seen stays clear, through how much the haze lies as thick as it gets, and how thick that is,
/// never so thick that what stands in it loses its shape; how much nearer saturated air and a
/// cloudburst round the eye close it in, each a share of the way; and how thick the low air the
/// sun's shafts show in stands in it round the eye, dry and saturated, dry air thin so the shafts
/// show only where the light breaks through.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Fog {
    pub clear: Meters,
    pub thick: Meters,
    pub thickest: f32,
    pub moist_closer: f32,
    pub rain_closer: f32,
    pub dry_air: f32,
    pub moist_air: f32,
}

impl Fog {
    /// This fog lying from `low` to `high` over the ground, `height` deep, as it hazes what is
    /// seen through it in air as moist as `humidity` while it `rains` round the eye (0 none to 1
    /// a cloudburst): the moister and the harder it rains, the nearer it closes in.
    pub fn lying(
        &self,
        low: Vec2,
        high: Vec2,
        height: Meters,
        humidity: f32,
        rains: f32,
    ) -> HazeBox {
        let closer =
            (1.0 - self.moist_closer * humidity) * (1.0 - self.rain_closer * rains.clamp(0.0, 1.0));
        HazeBox {
            low,
            high,
            height: height.0,
            clear: self.clear.0 * closer,
            thick: self.thick.0 * closer,
            thickest: self.thickest,
        }
    }

    /// How thick the low air the sun's shafts show in stands in this fog, as moist as `humidity`.
    pub fn air(&self, humidity: f32) -> f32 {
        self.dry_air + (self.moist_air - self.dry_air) * humidity.clamp(0.0, 1.0)
    }
}

/// The colour of the haze under a `sun` and a `sky`: the colour of the sky at the horizon, pale
/// blue by day, glowing at dusk and blue by night, greying as the air moistens to `humidity` and
/// as it `rains` round the eye (0 none to 1 a cloudburst), as bright as the world is `lit` of its
/// noon light, or black under a dark sky, so what stands far off in it sinks toward the sky.
pub fn haze(
    sun: Option<SunLight>,
    lit: f32,
    sky: Sky,
    humidity: f32,
    rains: f32,
    look: &SkyLook,
) -> Color {
    match (sky, sun) {
        (Sky::Dark, _) => Color::BLACK,
        (Sky::Air, None) => look.sunless_haze.color(),
        (Sky::Air, Some(sun)) => {
            let day = smoothstep(look.sun_gone.0, look.sun_full.0, sun.elevation.0);
            let glow = (1.0 - sun.elevation.0.abs() / 15.0).clamp(0.0, 1.0);
            let moist = mix(look.day_haze.color(), look.moist_haze.color(), humidity);
            let clear = mix(moist, look.rain_haze.color(), rains.clamp(0.0, 1.0));
            let hue = mix(
                mix(look.night_haze.color(), clear, day),
                look.dusk_haze.color(),
                glow,
            )
            .to_linear();
            Color::LinearRgba(hue * lit)
        }
    }
}

/// How moist the air of an area is under `clouds` holding `water`, 0 dry to 1 saturated: the
/// more water the sky holds, the more the air does, and cloud of any kind means some.
pub fn humidity(clouds: f32, water: f32, look: &SkyLook) -> f32 {
    let dry = look.moisture_of_cloud;
    (clouds * (dry + (1.0 - dry) * water)).clamp(0.0, 1.0)
}

/// An area's authored sun as it shines at `daylight` under `clouds` holding `water`: fixed, as
/// written; at an hour, climbing from the east over the morning to its authored noon height and
/// bearing and setting over the evening, reddening as it nears the horizon and going out beneath
/// it.
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
            let warmth = 1.0 - smoothstep(0.0, look.sky.reddens_below.0, elevation.0);
            SunLight {
                elevation,
                azimuth,
                color: mix(
                    color,
                    look.sky.setting_sun.color(),
                    warmth * look.sky.reddening,
                ),
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
    let night = look.sky.night_ambient;
    daytime(daylight, elevation, &look.sky).map_or(1.0, |lit| night + (1.0 - night) * lit)
        * (1.0 - look.sky.cloud_dimming * clouds::shade(water, &look.clouds) * clouds)
}

/// How far into the day a sun at `elevation` is, 0 once the night has fallen and 1 before dusk
/// begins, or nothing when the light stands fixed or there is no sun to move.
fn daytime(daylight: Daylight, elevation: Option<Degrees>, look: &SkyLook) -> Option<f32> {
    match (daylight, elevation) {
        (Daylight::Hour(_), Some(elevation)) => Some(smoothstep(
            look.night_falls.0,
            look.dusk_begins.0,
            elevation.0,
        )),
        _ => None,
    }
}

fn mix(from: Color, to: Color, by: f32) -> Color {
    Color::LinearRgba(from.to_linear().mix(&to.to_linear(), by))
}

/// A camera the sky lights: where its eye stands, how it sees the air, and whether its light is
/// being filtered.
type SkyLit = (
    Entity,
    &'static mut SkyLight,
    &'static GlobalTransform,
    Ref<'static, AtmosphereSettings>,
    Option<&'static GeneratedEnvironmentMapLight>,
);

/// A camera's sky light as it is kept: how many frames it has been worked out for with nothing
/// compiling, what shed the sky the frame before and for how many frames it has, what shed the
/// one last filtered, none while that is not known to be whole, and the filter's source for the
/// frames it rests.
#[derive(Component, Default)]
pub(super) struct SkyLight {
    warmth: u8,
    seen: Option<Shedding>,
    rested: u8,
    filtered: Option<Shedding>,
    source: Option<GeneratedEnvironmentMapLight>,
}

/// What sheds a sky's light on an eye: every sun's way and radiance, and every air with how far
/// from its middle the eye stands, in its own metres.
#[derive(Clone, PartialEq)]
struct Shedding {
    suns: Vec<(Vec3, Vec3)>,
    airs: Vec<(Entity, f32)>,
}

impl Shedding {
    fn seen_from(
        eye: Vec3,
        suns: &Query<(&DirectionalLight, &GlobalTransform)>,
        airs: &Query<(Entity, Ref<Atmosphere>, Ref<GlobalTransform>)>,
    ) -> Self {
        Self {
            suns: suns
                .iter()
                .map(|(sun, placed)| {
                    (
                        placed.forward().as_vec3(),
                        sun.color.to_linear().to_vec3() * sun.illuminance,
                    )
                })
                .collect(),
            airs: airs
                .iter()
                .map(|(air, _, placed)| {
                    (
                        air,
                        placed.affine().inverse().transform_point3(eye).length(),
                    )
                })
                .collect(),
        }
    }

    fn far_from(&self, other: &Self) -> bool {
        let turned = SUN_TURNED.radians().0.cos();
        self.suns.len() != other.suns.len()
            || self.airs.len() != other.airs.len()
            || self.suns.iter().zip(&other.suns).any(|(a, b)| {
                a.0.dot(b.0) < turned
                    || (a.1 - b.1).abs().max_element() > SUN_BRIGHTENED * a.1.max(b.1).max_element()
            })
            || self
                .airs
                .iter()
                .zip(&other.airs)
                .any(|(a, b)| a.0 != b.0 || (a.1 - b.1).abs() > AIR_CLIMBED)
    }
}
