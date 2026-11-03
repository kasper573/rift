//! The weather a ground's cover moves by: the shown area's climate and the bodies treading
//! through, published each frame in the picture every cover's shader reads. Every body treads
//! through the cover on its limbs, as its skeleton moves them: a cover bends round the lowest
//! limbs of the bodies nearest the camera's subject.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use super::uniform::{LIMBS_PER_TREAD, TREADS, Tread, TreadingLimb, Weather, WeatherImage};
use crate::core::render::CameraRig;
use crate::core::render::world::body::Body;
use crate::core::render::world::sky::Daylight;

/// The weather over the shown area as the client draws it: the wind as a direction over the
/// ground (x east, y south) scaled by its strength, from calm at 0 to a gale at 1, how much cloud
/// covers the sky and how much water the cloud holds.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct Climate {
    pub wind: Vec2,
    pub clouds: f32,
    pub water: f32,
}

/// What the weather is made of: the moment, the shown area's climate and daylight, where the camera
/// looks and who treads the ground.
#[derive(SystemParam)]
pub(super) struct WeatherSources<'w, 's> {
    time: Res<'w, Time>,
    climate: Option<Res<'w, Climate>>,
    daylight: Option<Res<'w, Daylight>>,
    rig: Option<Res<'w, CameraRig>>,
    bodies: Query<'w, 's, (&'static GlobalTransform, &'static Body)>,
    joints: Query<'w, 's, &'static GlobalTransform>,
}

/// Every frame the covers learn the wind, the moment and who treads through them.
pub(super) fn blow(
    sources: WeatherSources,
    picture: Res<WeatherImage>,
    mut images: ResMut<Assets<Image>>,
    mut last: Local<Weather>,
) {
    let WeatherSources {
        time,
        climate,
        daylight,
        rig,
        bodies,
        joints,
    } = sources;
    let subject = rig.map_or(Vec3::ZERO, |rig| rig.target());
    let mut nearest: Vec<(f32, &GlobalTransform, &Body)> = bodies
        .iter()
        .filter(|(_, body)| !body.limbs().is_empty())
        .map(|(placed, body)| (placed.translation().distance_squared(subject), placed, body))
        .collect();
    nearest.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut weather = Weather {
        treads: nearest
            .into_iter()
            .take(TREADS)
            .map(|(_, placed, body)| tread(placed, body, &joints))
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
    }
    weather.time = time.elapsed_secs();
    if *last == weather {
        return;
    }
    *last = weather.clone();
    if let Some(mut image) = images.get_mut(&picture.0) {
        image.data = Some(weather.texels());
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
    }
}

fn lowest(limb: &TreadingLimb) -> f32 {
    limb.from.y.min(limb.to.y) - limb.girth
}
