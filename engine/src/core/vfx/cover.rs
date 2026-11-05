//! The weather the world's shaders read: the shown area's weather, the rain over the ground round
//! the camera's subject and the bodies treading through the cover, published each frame in the
//! picture they read. Every body treads
//! through the cover on its limbs, as its skeleton moves them: a cover bends round the lowest
//! limbs of the bodies nearest the camera's subject.

use bevy::ecs::system::SystemParam;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy::render::extract_resource::ExtractResource;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::{
    Extent3d, Origin3d, TexelCopyBufferLayout, TexelCopyTextureInfo, TextureAspect, TextureId,
};
use bevy::render::renderer::RenderQueue;
use bevy::render::texture::GpuImage;

use super::rain::{RainField, Shelter};
use super::uniform::{
    LIMBS_PER_TREAD, PICTURE_WIDTH, TRAIL_EVERY, TRAIL_POINTS, TREADS, Tread, TreadingLimb,
    Weather, WeatherImage, texel_bytes,
};
use crate::core::render::CameraRig;
use crate::core::render::sample::UploadedBytes;
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

/// The texels of the weather's picture that changed since it was last written, run by run: where
/// each run starts and what it holds now.
#[derive(Resource, Clone, Default, ExtractResource)]
#[extract_app(RenderApp)]
pub(super) struct WeatherWrites(Vec<(usize, Vec<Vec4>)>);

/// Every frame the covers learn the wind and who treads through them, and where each treading
/// body stood lately: only what changed of it is written.
pub(super) fn blow(
    sources: WeatherSources,
    mut writes: ResMut<WeatherWrites>,
    mut last: Local<Option<Vec<Vec4>>>,
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
    // On the clock the shaders read the time by, so a trail's age is the frame's time less when
    // it was laid.
    let now = time.elapsed_secs_wrapped();
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
    let texels = weather.texels(&rain, &shelter);
    let was = last.get_or_insert_with(Weather::still);
    let runs = changed_runs(was, &texels);
    *was = texels;
    if !runs.is_empty() {
        writes.0 = runs;
    }
}

/// Writes what changed of the weather into its picture on the GPU, or all of it into a picture
/// the GPU holds anew.
pub(super) fn write_weather(
    writes: Res<WeatherWrites>,
    picture: Option<Res<WeatherImage>>,
    images: Res<RenderAssets<GpuImage>>,
    queue: Res<RenderQueue>,
    mut uploaded: ResMut<UploadedBytes>,
    mut held: Local<Option<Vec<Vec4>>>,
    mut written: Local<Option<TextureId>>,
) {
    let held = held.get_or_insert_with(Weather::still);
    if writes.is_changed() {
        for (start, run) in &writes.0 {
            held[*start..*start + run.len()].copy_from_slice(run);
        }
    }
    let Some(gpu) = picture.and_then(|picture| images.get(&picture.0)) else {
        return;
    };
    if *written != Some(gpu.texture.id()) {
        *written = Some(gpu.texture.id());
        uploaded.0 += write_texels(&queue, gpu, 0, held);
    } else if writes.is_changed() {
        for (start, run) in &writes.0 {
            uploaded.0 += write_texels(&queue, gpu, *start, run);
        }
    }
}

/// Writes `texels` into the picture from the texel `start` on, a row at a time, returning the
/// bytes written.
fn write_texels(queue: &RenderQueue, gpu: &GpuImage, start: usize, texels: &[Vec4]) -> u64 {
    let mut at = start;
    let mut rest = texels;
    let mut bytes = 0;
    while !rest.is_empty() {
        let (row, column) = (at / PICTURE_WIDTH, at % PICTURE_WIDTH);
        let (line, after) = rest.split_at(rest.len().min(PICTURE_WIDTH - column));
        let data = texel_bytes(line);
        queue.write_texture(
            TexelCopyTextureInfo {
                texture: &gpu.texture,
                mip_level: 0,
                origin: Origin3d {
                    x: column as u32,
                    y: row as u32,
                    z: 0,
                },
                aspect: TextureAspect::All,
            },
            &data,
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(data.len() as u32),
                rows_per_image: None,
            },
            Extent3d {
                width: line.len() as u32,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        bytes += data.len() as u64;
        at += line.len();
        rest = after;
    }
    bytes
}

/// The runs of texels `now` holds that differ from `was`, joined across short stretches alike.
fn changed_runs(was: &[Vec4], now: &[Vec4]) -> Vec<(usize, Vec<Vec4>)> {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for (index, (before, after)) in was.iter().zip(now).enumerate() {
        if before == after {
            continue;
        }
        match runs.last_mut() {
            Some((_, end)) if index - *end <= JOINED_ACROSS => *end = index + 1,
            _ => runs.push((index, index + 1)),
        }
    }
    runs.into_iter()
        .map(|(start, end)| (start, now[start..end].to_vec()))
        .collect()
}

/// How many unchanged texels a run of changed ones is carried across rather than broken at: a
/// write costs more than the few texels it would leave out.
const JOINED_ACROSS: usize = 4;

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
