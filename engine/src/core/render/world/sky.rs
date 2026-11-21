//! The sky over the world and the daylight it stands in: an atmosphere scattering the sun's and
//! the moon's light, which paints the sky, blues the distance, reddens a low sun and lights
//! everything from above, as moist as the weather makes the air, or the dark of a cave or a
//! stage; the air the sun's shafts show in, and how an area's authored sun and ambient light
//! turn with the hour of a day cycle and dim under cloud, the full moon standing opposite the
//! sun and the eye opening to the dark.

use std::f32::consts::PI;

use bevy::light::atmosphere::{Falloff, PhaseFunction, ScatteringMedium, ScatteringTerm};
use bevy::light::{
    Atmosphere, AtmosphereEnvironmentMapLight, EnvironmentMapLight, FogVolume,
    GeneratedEnvironmentMapLight,
};
use bevy::pbr::AtmosphereSettings;
use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{DRAWN_FRAMES_AFTER, RenderProgress, clouds, lights};
use crate::core::math::{Rect, Rgb, smoothstep};
use crate::core::render::Look;
use crate::core::units::{Degrees, HourOfDay, Meters, Stops};

/// How many points of each slice of the air between the eye and the farthest it sees the air's
/// light is summed over: each ray sums its points one after the other, so the GPU's time on the
/// air is the points' count however few the rays.
const AIR_SAMPLES: u32 = 4;
/// How high the air stands over the ground, which every height below is a share of.
const AIR_HEIGHT: f32 = 100_000.0;
/// What the gas of the air scatters of each colour a metre, thinning by a factor of e every
/// `GAS_THINS` metres up: the blue of the sky and of the distance (Hillaire 2020, as below).
const GAS_SCATTERS: Vec3 = Vec3::new(5.802e-6, 13.558e-6, 33.1e-6);
const GAS_THINS: f32 = 8_000.0;
/// What the dust and droplets of a clear day scatter and swallow a metre, alike of every
/// colour and mostly onward the way the light was going: the white of the horizon and the glow
/// round the sun.
const DUST_SCATTERS: f32 = 3.996e-6;
const DUST_SWALLOWS: f32 = 0.444e-6;
const DUST_THINS: f32 = 1_200.0;
const DUST_ONWARD: f32 = 0.8;
/// What the ozone swallows of each colour a metre where it lies thickest, `OZONE_HIGH` up, in a
/// layer `OZONE_DEEP` deep: the blue the zenith keeps through twilight.
const OZONE_SWALLOWS: Vec3 = Vec3::new(0.650e-6, 1.881e-6, 0.085e-6);
const OZONE_HIGH: f32 = 25_000.0;
const OZONE_DEEP: f32 = 30_000.0;
/// How many steps the air's moisture is told apart by: the GPU makes the air's medium anew
/// whenever it changes, so it follows the weather in steps, each too small to see.
const MOISTURE_STEPS: f32 = 16.0;
/// How many stretches a light's way down through the air is summed over.
const AIR_STEPS: u32 = 48;
/// How many ways up from the horizon the air tells a shader how much of itself a light from
/// beyond it crosses, each climbing the square of its share of the way to straight up, so they
/// crowd toward the horizon, where the air it crosses changes fastest.
pub const AIR_WAYS: usize = 16;
/// How bright the eye takes each colour of a light for.
const LUMINANCE: Vec3 = Vec3::new(0.2126, 0.7152, 0.0722);
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
    /// How much dust dry air holds, of an Earth's on a clear day: the haze that whitens the
    /// horizon and glows round the sun.
    pub dust: f32,
    /// The colour of the land past the world's edge, which the far air stands over.
    pub far_ground: Rgb,
    /// How moist cloud of no water at all leaves the air, of saturated.
    pub moisture_of_cloud: f32,
    /// How much of the light a face under a clear sky gets comes from the sky rather than the sun.
    pub sky_share: f32,
    /// How much the sky lights everything from above, of the atmosphere's own brightness.
    pub sky_light: f32,
    /// How many degrees across the sun's and the moon's discs are.
    pub sun_size: Degrees,
    pub moon_size: Degrees,
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
/// behind the world, hazing the distance as far as `farthest`, the farthest the camera sees
/// anything, and shedding its light from above, or none from the dark.
/// The camera keeps the means to draw the air for as long as it lives and only their strength
/// and reach change from sky to sky, since making them anew fills the GPU's command memory with
/// the work of making them, which it keeps for good.
pub fn show(world: &mut World, camera: Entity, sky: Sky, farthest: Meters) {
    let Some(look) = world.get_resource::<Look>().map(|look| look.sky) else {
        return;
    };
    let sky_light = look.sky_light;
    // The air is told its distances in its own metres.
    let reach = farthest.0 * look.air_per_metre;
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
                aerial_view_lut_max_distance: reach,
                aerial_view_lut_samples: AIR_SAMPLES,
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
    if let Some(mut settings) = camera.get_mut::<AtmosphereSettings>()
        && settings.aerial_view_lut_max_distance != reach
    {
        settings.aerial_view_lut_max_distance = reach;
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
    Hour(HourOfDay),
}

impl Daylight {
    /// The hour as a share of the day, for a shader; noon when fixed.
    pub fn share(self) -> f32 {
        match self {
            Daylight::Fixed => 0.5,
            Daylight::Hour(hour) => hour.share(),
        }
    }
}

/// A sun or a moon as it shines at some hour: where it stands and how.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SunLight {
    pub elevation: Degrees,
    pub azimuth: Degrees,
    pub color: Color,
    pub illuminance: f32,
}

impl SunLight {
    /// Whether this light has turned or changed its strength from `other` by as much as the
    /// sky's light takes for a change: what follows a light that is always moving redoes its
    /// work only then.
    pub fn far_from(&self, other: &SunLight) -> bool {
        let shed = |light: &SunLight| {
            (
                lights::toward(light.elevation, light.azimuth),
                light.color.to_linear().to_vec3() * light.illuminance,
            )
        };
        shed_apart(shed(self), shed(other))
    }
}

/// The air over the world under `sky`: an Earth's atmosphere whose dust scatters the light
/// `scattering` times as much as dry air's, the water in moist air scattering it white and
/// hazing the horizon the more it holds, scaled down so the world's few kilometres stretch over
/// enough air to haze, over a far ground of the look's colour; or over the dark, an air that
/// scatters nothing over a black ground.
pub fn atmosphere(
    media: &mut Assets<ScatteringMedium>,
    sky: Sky,
    scattering: f32,
    look: &SkyLook,
) -> impl Bundle {
    let mut atmosphere = Atmosphere::earth(media.add(medium(sky, scattering, look)));
    atmosphere.outer_radius = atmosphere.inner_radius + AIR_HEIGHT;
    atmosphere.ground_albedo = match sky {
        Sky::Air => look.far_ground.color().to_linear().to_vec3(),
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
        Moistened(scattering),
    )
}

/// The lights of a sky as they shine at some hour: its sun, and where the day turns the full
/// moon, which always stands opposite it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyLights {
    pub sun: SunLight,
    pub moon: Option<SunLight>,
}

impl SkyLights {
    /// The lights of an area whose authored sun stands `noon_elevation` high at `noon_azimuth`
    /// in `color`, as they shine above the air at `daylight` under `clouds` holding `water`.
    pub fn at(
        daylight: Daylight,
        noon_elevation: Degrees,
        noon_azimuth: Degrees,
        color: Color,
        clouds: f32,
        water: f32,
        look: &Look,
    ) -> SkyLights {
        let sun = sun_at(
            daylight,
            noon_elevation,
            noon_azimuth,
            color,
            clouds,
            water,
            look,
        );
        SkyLights {
            sun,
            moon: match daylight {
                Daylight::Fixed => None,
                Daylight::Hour(_) => Some(moon_of(sun, clouds, water, look)),
            },
        }
    }

    /// These lights as they reach a point `height` over the air's floor through `air`.
    pub fn landed(&self, air: &SkyAir, height: Meters, look: &SkyLook) -> SkyLights {
        SkyLights {
            sun: air.landed(self.sun, look.sun_size, height),
            moon: self
                .moon
                .map(|moon| air.landed(moon, look.moon_size, height)),
        }
    }

    /// Whether the sun is the stronger of these lights: of the lights as they have landed, the
    /// stronger casts the world's shadows.
    pub fn sun_stronger(&self) -> bool {
        self.moon
            .is_none_or(|moon| self.sun.illuminance >= moon.illuminance)
    }

    /// The sun when `sun_casts`, or else the moon, and the other of the two.
    pub fn caster(&self, sun_casts: bool) -> (SunLight, Option<SunLight>) {
        match (sun_casts, self.moon) {
            (false, Some(moon)) => (moon, Some(self.sun)),
            _ => (self.sun, self.moon),
        }
    }

    /// Every one of these lights.
    pub fn all(&self) -> impl Iterator<Item = SunLight> {
        std::iter::once(self.sun).chain(self.moon)
    }
}

/// The air as a light crosses it on its way down to the world.
pub struct SkyAir {
    medium: ScatteringMedium,
    floor: f32,
    per_metre: f32,
}

impl SkyAir {
    /// The air under `sky`, scattering the light `scattering` times as much as dry air's dust.
    pub fn new(sky: Sky, scattering: f32, look: &SkyLook) -> SkyAir {
        SkyAir {
            medium: medium(sky, scattering, look),
            floor: Atmosphere::earth(Handle::default()).inner_radius,
            per_metre: look.air_per_metre,
        }
    }

    /// `light` as it reaches a point `height` over the air's floor, its disc `disc` across:
    /// dimmed and reddened by the air it crossed, the more the lower it stands, and gone once
    /// the ground has hidden all of its disc.
    pub fn landed(&self, light: SunLight, disc: Degrees, height: Meters) -> SunLight {
        let from = self.from(height);
        let horizon = -(self.floor / from).clamp(-1.0, 1.0).acos();
        let elevation = light.elevation.radians().0;
        let across = disc.radians().0.max(f32::EPSILON);
        let showing = ((elevation - horizon) / across + 0.5).clamp(0.0, 1.0);
        let through = self.through(from, elevation.max(horizon + 1e-4));
        let kept = through.dot(LUMINANCE);
        let color = light.color.to_linear().to_vec3() * through / kept.max(f32::EPSILON);
        SunLight {
            color: Color::linear_rgb(color.x, color.y, color.z),
            illuminance: light.illuminance * kept * showing,
            ..light
        }
    }

    /// How much air a light from beyond it crosses down to a point `height` over the air's
    /// floor, for each colour, along each of `AIR_WAYS` ways up from the horizon.
    pub fn crossed_to(&self, height: Meters) -> [Vec3; AIR_WAYS] {
        let from = self.from(height);
        std::array::from_fn(|way| {
            let up = (way as f32 / (AIR_WAYS - 1) as f32).powi(2);
            self.crossed(from, up.asin())
        })
    }

    fn from(&self, height: Meters) -> f32 {
        self.floor + (height.0 * self.per_metre).max(1.0)
    }

    /// How much of each colour of a light standing `elevation` radians over the horizon
    /// crosses the air down to `from` metres from the middle of the world it lies round.
    fn through(&self, from: f32, elevation: f32) -> Vec3 {
        (-self.crossed(from, elevation)).exp()
    }

    fn crossed(&self, from: f32, elevation: f32) -> Vec3 {
        let up = elevation.sin();
        let top = self.floor + AIR_HEIGHT;
        let way = -from * up + (from * from * (up * up - 1.0) + top * top).max(0.0).sqrt();
        let mut crossed = Vec3::ZERO;
        for step in 0..AIR_STEPS {
            // Finer steps near the ground, where the air is thick.
            let (near, far) = (
                (step as f32 / AIR_STEPS as f32).powi(2) * way,
                ((step + 1) as f32 / AIR_STEPS as f32).powi(2) * way,
            );
            let along = (near + far) / 2.0;
            let high = (from * from + along * along + 2.0 * from * along * up).sqrt() - self.floor;
            let share = 1.0 - high / AIR_HEIGHT;
            for term in &self.medium.terms {
                crossed +=
                    (term.absorption + term.scattering) * term.falloff.sample(share) * (far - near);
            }
        }
        crossed
    }
}

/// How moist the air's medium is made for air as moist as `humidity`.
pub fn stepped_humidity(humidity: f32) -> f32 {
    (humidity.clamp(0.0, 1.0) * MOISTURE_STEPS).round() / MOISTURE_STEPS
}

/// Has the air of `atmosphere` scatter the light `scattering` times as much as dry air's dust,
/// when it doesn't already.
pub fn moisten(world: &mut World, atmosphere: Entity, sky: Sky, scattering: f32, look: &SkyLook) {
    let Some(mut moistened) = world.get_mut::<Moistened>(atmosphere) else {
        return;
    };
    if moistened.0 == scattering {
        return;
    }
    moistened.0 = scattering;
    let Some(handle) = world
        .get::<Atmosphere>(atmosphere)
        .map(|air| air.medium.clone())
    else {
        return;
    };
    if let Some(mut held) = world
        .resource_mut::<Assets<ScatteringMedium>>()
        .get_mut(&handle)
    {
        *held = medium(sky, scattering, look);
    }
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

/// How moist the air of an area is under `clouds` holding `water`, 0 dry to 1 saturated: the
/// more water the sky holds, the more the air does, and cloud of any kind means some.
pub fn humidity(clouds: f32, water: f32, look: &SkyLook) -> f32 {
    let dry = look.moisture_of_cloud;
    (clouds * (dry + (1.0 - dry) * water)).clamp(0.0, 1.0)
}

/// An area's authored sun as it shines above the air at `daylight`, under `clouds` holding
/// `water`: fixed, as written; at an hour, climbing from the east over the morning to its
/// authored noon height and bearing, setting over the evening and going on round under the
/// world. The air it shines through dims and reddens it on the way down ([`SkyAir::landed`]).
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
            SunLight {
                elevation,
                azimuth,
                color,
                illuminance: sun_lux * clouded,
            }
        }
    }
}

/// Where a sun standing `noon_elevation` high at `noon_azimuth` at noon stands at `hour`: on the
/// great circle through its noon place that rises due a quarter turn before its noon bearing and
/// sets a quarter turn after it, turned a twenty-fourth of the way round it each hour, as a sky
/// turns round the world.
fn orbit(noon_elevation: Degrees, noon_azimuth: Degrees, hour: HourOfDay) -> (Degrees, Degrees) {
    let noon = lights::toward(noon_elevation, noon_azimuth);
    let setting = lights::toward(Degrees(0.0), Degrees(noon_azimuth.0 + 90.0));
    let (along, across) = ((hour.0 - 12.0) / 12.0 * PI).sin_cos();
    let way = noon * across + setting * along;
    let elevation = way.y.clamp(-1.0, 1.0).asin();
    let azimuth = way.x.atan2(-way.z);
    (
        Degrees(elevation.to_degrees()),
        Degrees(azimuth.to_degrees().rem_euclid(360.0)),
    )
}

/// How the sky has turned at `daylight` over an area whose sun stands `noon_elevation` high at
/// `noon_azimuth` at noon: what carries a way of the world to the way of the sky's own it looks
/// along. The sky turns with its sun, about the pole of the sun's orbit, and stands as at noon
/// where the light stands fixed.
pub fn sky_turn(daylight: Daylight, noon_elevation: Degrees, noon_azimuth: Degrees) -> Mat3 {
    match daylight {
        Daylight::Fixed => Mat3::IDENTITY,
        Daylight::Hour(hour) => {
            let noon = lights::toward(noon_elevation, noon_azimuth);
            let setting = lights::toward(Degrees(0.0), Degrees(noon_azimuth.0 + 90.0));
            Mat3::from_axis_angle(
                noon.cross(setting).normalize(),
                -(hour.0 - 12.0) / 12.0 * PI,
            )
        }
    }
}

/// The moon as it shines above the air while `sun` shines, under `clouds` holding `water`:
/// always full, standing opposite the sun.
pub fn moon_of(sun: SunLight, clouds: f32, water: f32, look: &Look) -> SunLight {
    SunLight {
        elevation: Degrees(-sun.elevation.0),
        azimuth: Degrees((sun.azimuth.0 + 180.0).rem_euclid(360.0)),
        color: look.sky.moonlight.color(),
        illuminance: look.light.sun_lux * look.sky.moon_share * clouded(clouds, water, look),
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

/// How much wider the eye opens at `daylight` under the lights `landed` on the ground, as an eye
/// opens to the dusk and the night: by `adapting` for every stop the light falls, the sky dimming
/// too as the lights stand lower, up to `most_adapted`; never under a fixed light, which stands
/// as authored.
pub fn adaptation(
    daylight: Daylight,
    landed: impl Iterator<Item = SunLight>,
    look: &Look,
) -> Stops {
    match daylight {
        Daylight::Fixed => Stops(0.0),
        Daylight::Hour(_) => {
            let strength: f32 = landed
                .map(|light| {
                    let height = light.elevation.radians().0.sin().max(0.0).sqrt();
                    light.illuminance / look.light.sun_lux * (0.3 + 0.7 * height)
                })
                .sum();
            let opened = -strength.max(1e-3).log2() * look.sky.adapting;
            Stops(opened.clamp(0.0, look.sky.most_adapted.0))
        }
    }
}

/// How much of the sun's light reaches the ground through `clouds` holding `water`.
pub fn clouded(clouds: f32, water: f32, look: &Look) -> f32 {
    1.0 - clouds::shade(water, &look.clouds) * clouds
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

/// What the air under `sky` is made of: the gas, the dust with the droplets of air scattering
/// `scattering` times as much as dry air's, and the ozone, each thinning with height as it does
/// over an Earth; nothing under the dark.
fn medium(sky: Sky, scattering: f32, look: &SkyLook) -> ScatteringMedium {
    let terms = match sky {
        Sky::Dark => Vec::new(),
        Sky::Air => vec![
            ScatteringTerm {
                absorption: Vec3::ZERO,
                scattering: GAS_SCATTERS,
                falloff: Falloff::Exponential {
                    scale: GAS_THINS / AIR_HEIGHT,
                },
                phase: PhaseFunction::Rayleigh,
            },
            ScatteringTerm {
                absorption: Vec3::splat(DUST_SWALLOWS * look.dust),
                scattering: Vec3::splat(DUST_SCATTERS * look.dust * scattering),
                falloff: Falloff::Exponential {
                    scale: DUST_THINS / AIR_HEIGHT,
                },
                phase: PhaseFunction::Mie {
                    asymmetry: DUST_ONWARD,
                },
            },
            ScatteringTerm {
                absorption: OZONE_SWALLOWS,
                scattering: Vec3::ZERO,
                falloff: Falloff::Tent {
                    center: 1.0 - OZONE_HIGH / AIR_HEIGHT,
                    width: OZONE_DEEP / AIR_HEIGHT,
                },
                phase: PhaseFunction::Isotropic,
            },
        ],
    };
    let earth = ScatteringMedium::default();
    ScatteringMedium::new(earth.falloff_resolution, earth.phase_resolution, terms)
}

/// How many times dry air's dust the air of an atmosphere scatters the light.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
struct Moistened(f32);

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
        self.suns.len() != other.suns.len()
            || self.airs.len() != other.airs.len()
            || self
                .suns
                .iter()
                .zip(&other.suns)
                .any(|(a, b)| shed_apart(*a, *b))
            || self
                .airs
                .iter()
                .zip(&other.airs)
                .any(|(a, b)| a.0 != b.0 || (a.1 - b.1).abs() > AIR_CLIMBED)
    }
}

/// Whether two lights, each the way it lies and its radiance, differ by as much as the sky's
/// light takes for a change.
fn shed_apart(a: (Vec3, Vec3), b: (Vec3, Vec3)) -> bool {
    a.0.dot(b.0) < SUN_TURNED.radians().0.cos()
        || (a.1 - b.1).abs().max_element() > SUN_BRIGHTENED * a.1.max(b.1).max_element()
}
