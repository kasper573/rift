//! The weather the world's shaders read: the shown area's weather, the rain over the ground round
//! the camera's subject and the bodies treading through the cover, published each frame in the
//! picture they read. Every body treads
//! through the cover on its limbs, as its skeleton moves them: a cover bends round the lowest
//! limbs of the bodies nearest the camera's subject.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use bevy::platform::collections::HashMap;

use super::rain::{RainField, Shelter};
use super::uniform::{
    LIMBS_PER_TREAD, TRAIL_EVERY, TRAIL_POINTS, TREADS, Tread, TreadingLimb, Weather, WeatherImage,
};
use crate::core::render::CameraRig;
use crate::core::render::world::body::Body;
use crate::core::render::world::sky::Daylight;

/// The weather over the shown area as the client draws it: the wind as a direction over the
/// ground (x east, y south) scaled by its strength, from calm at 0 to a gale at 1, how much cloud
/// covers the sky, how much water the cloud holds, how hard it rains under the wettest, 0 none to
/// 1 a cloudburst, and how brightly lightning lights the world this moment, 0 none to 1 a strike
/// close by.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct ShownWeather {
    pub wind: Vec2,
    pub clouds: f32,
    pub water: f32,
    pub rain: f32,
    pub flash: f32,
}

/// What the weather is made of: the moment, the shown area's weather, its rain and daylight, where
/// the camera looks and who treads the ground.
#[derive(SystemParam)]
pub(super) struct WeatherSources<'w, 's> {
    time: Res<'w, Time>,
    climate: Option<Res<'w, ShownWeather>>,
    rain: Res<'w, RainField>,
    shelter: Res<'w, Shelter>,
    daylight: Option<Res<'w, Daylight>>,
    rig: Option<Res<'w, CameraRig>>,
    bodies: Query<'w, 's, (Entity, &'static GlobalTransform, &'static Body)>,
    joints: Query<'w, 's, &'static GlobalTransform>,
}

/// Every frame the covers learn the wind, the moment and who treads through them, and where each
/// treading body stood lately.
pub(super) fn blow(
    sources: WeatherSources,
    picture: Res<WeatherImage>,
    mut images: ResMut<Assets<Image>>,
    mut last: Local<Weather>,
    mut trails: Local<HashMap<Entity, Vec<(Vec2, f32)>>>,
) {
    let WeatherSources {
        time,
        climate,
        rain,
        shelter,
        daylight,
        rig,
        bodies,
        joints,
    } = sources;
    let subject = rig.map_or(Vec3::ZERO, |rig| rig.target());
    let now = time.elapsed_secs();
    let mut nearest: Vec<(f32, Entity, &GlobalTransform, &Body)> = bodies
        .iter()
        .filter(|(_, _, body)| !body.limbs().is_empty())
        .map(|(entity, placed, body)| {
            (
                placed.translation().distance_squared(subject),
                entity,
                placed,
                body,
            )
        })
        .collect();
    nearest.sort_by(|a, b| a.0.total_cmp(&b.0));
    nearest.truncate(TREADS);
    trails.retain(|entity, _| nearest.iter().any(|(_, near, _, _)| near == entity));
    let mut weather = Weather {
        treads: nearest
            .into_iter()
            .map(|(_, entity, placed, body)| {
                let trail = trails.entry(entity).or_default();
                lay_trail(trail, placed.translation().xz(), now);
                Tread {
                    trail: trail.clone(),
                    ..tread(placed, body, &joints)
                }
            })
            .collect(),
        ..Weather::default()
    };
    if let Some(daylight) = daylight {
        weather.daylight = daylight.share();
    }
    if let Some(climate) = climate {
        weather.clouds = climate.clouds;
        weather.water = climate.water;
        weather.wind = climate.wind;
        weather.rain = climate.rain;
        weather.flash = climate.flash;
    }
    weather.time = now;
    if *last == weather && !rain.is_changed() && !shelter.is_changed() {
        return;
    }
    *last = weather.clone();
    if let Some(mut image) = images.get_mut(&picture.0) {
        image.data = Some(weather.texels(&rain, &shelter));
    }
}

/// The body as it treads: its lowest limbs where its joints stand now, and how far from where it
/// stands they reach.
fn tread(placed: &GlobalTransform, body: &Body, joints: &Query<&GlobalTransform>) -> Tread {
    let mut limbs: Vec<TreadingLimb> = body
        .limbs()
        .iter()
        .filter_map(|limb| {
            let joint = joints.get(limb.joint).ok()?;
            Some(TreadingLimb {
                from: joint.translation(),
                to: joint.transform_point(limb.tail),
                girth: limb.girth * joint.scale().abs().max_element(),
            })
        })
        .collect();
    limbs.sort_by(|a, b| lowest(a).total_cmp(&lowest(b)));
    limbs.truncate(LIMBS_PER_TREAD);
    let ground = placed.translation().xz();
    let reach = limbs
        .iter()
        .map(|limb| {
            ground
                .distance(limb.from.xz())
                .max(ground.distance(limb.to.xz()))
                + limb.girth
        })
        .fold(0.0, f32::max);
    Tread {
        ground,
        reach,
        limbs,
        trail: Vec::new(),
    }
}

/// Lays where a body stands `at` at the head of its trail, every [`TRAIL_EVERY`] seconds, keeping
/// the newest [`TRAIL_POINTS`].
fn lay_trail(trail: &mut Vec<(Vec2, f32)>, at: Vec2, now: f32) {
    if trail
        .first()
        .is_some_and(|(_, laid)| now - laid < TRAIL_EVERY)
    {
        return;
    }
    trail.insert(0, (at, now));
    trail.truncate(TRAIL_POINTS);
}

fn lowest(limb: &TreadingLimb) -> f32 {
    limb.from.y.min(limb.to.y) - limb.girth
}
