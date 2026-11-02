//! The weather a ground's cover moves by: the shown area's climate and the bodies treading
//! through, published each frame in the picture every cover's shader reads.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use super::uniform::{TREADS, Weather, WeatherImage};
use crate::core::render::CameraRig;
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

/// A body that bends the cover it walks through, `radius` metres around its feet.
#[derive(Component, Clone, Copy, Debug)]
pub struct Treads {
    pub radius: f32,
}

/// What the weather is made of: the moment, the shown area's climate and daylight, where the camera
/// looks and who treads the ground.
#[derive(SystemParam)]
pub(super) struct Sky<'w, 's> {
    time: Res<'w, Time>,
    climate: Option<Res<'w, Climate>>,
    daylight: Option<Res<'w, Daylight>>,
    rig: Option<Res<'w, CameraRig>>,
    treading: Query<'w, 's, (&'static GlobalTransform, &'static Treads)>,
}

/// Every frame the covers learn the wind, the moment and who treads through them.
pub(super) fn blow(
    sky: Sky,
    picture: Res<WeatherImage>,
    mut images: ResMut<Assets<Image>>,
    mut last: Local<Weather>,
) {
    let Sky {
        time,
        climate,
        daylight,
        rig,
        treading,
    } = sky;
    let subject = rig.map_or(Vec3::ZERO, |rig| rig.target());
    let mut treads: Vec<(f32, Vec4)> = treading
        .iter()
        .map(|(transform, treads)| {
            let at = transform.translation();
            (
                at.distance_squared(subject),
                Vec4::new(at.x, at.z, treads.radius, 0.0),
            )
        })
        .collect();
    treads.sort_by(|a, b| a.0.total_cmp(&b.0));
    treads.truncate(TREADS);
    let mut weather = Weather::default();
    for (slot, (_, tread)) in treads.iter().enumerate() {
        weather.tread[slot] = *tread;
    }
    weather.treads = treads.len() as u32;
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
    *last = weather;
    if let Some(mut image) = images.get_mut(&picture.0) {
        image.data = Some(weather.texels());
    }
}
