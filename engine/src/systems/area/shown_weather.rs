//! The weather the world's shaders read: the shown area's weather, the rain over the ground round
//! the camera's subject and the bodies treading through the cover, published each frame on the
//! board they read. Every body treads through the cover on its limbs, as its skeleton moves them:
//! a cover bends round the lowest limbs of the bodies nearest the camera's subject.

use bevy::ecs::system::SystemParam;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::shader::Shader;

use super::heavens::{RainMaterial, SkyMaterial};
use super::rain::{RAIN_CELL, RAIN_SIDE, RainField, SHELTER_CELL, SHELTER_SIDE, Shelter};
use super::stages;
use crate::core::render::world::afloat::Afloat;
use crate::core::render::world::body::Body;
use crate::core::render::world::gpu::draw::{GROUND_LIT_SHADER, ground_lit_shader};
use crate::core::render::world::sky::Daylight;
use crate::core::render::{CameraRig, GroundHeights, ground_pos};
use crate::core::vfx::board::{BOARD_WIDTH, ShaderBoardPlugin, ShaderBoardWrites};

/// Lays the weather the world's shaders read on the board they read it from, and the materials
/// the sky and the rain are drawn with.
pub struct ShownWeatherPlugin;

impl Plugin for ShownWeatherPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            ShaderBoardPlugin {
                initial: WeatherBoard::still(),
            },
            MaterialPlugin::<SkyMaterial>::default(),
            MaterialPlugin::<RainMaterial>::default(),
        ))
        .init_resource::<RainField>()
        .init_resource::<Shelter>()
        .add_systems(Update, blow);
        let _ = app.world_mut().resource_mut::<Assets<Shader>>().insert(
            GROUND_LIT_SHADER.id(),
            ground_lit_shader(&stages::weather_helpers()),
        );
    }
}

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

/// A thing lying on the ground that the cover parts round as round a body's limb: `girth` round
/// where it stands, from the ground up to `height`. Bodies tread first, so among many a thing
/// may lie in standing cover.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct PartsCover {
    pub girth: f32,
    pub height: f32,
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
    ground: Option<Res<'w, GroundHeights>>,
    bodies: Query<'w, 's, (Entity, &'static GlobalTransform, &'static Body, Has<Afloat>)>,
    lying: Query<'w, 's, (&'static GlobalTransform, &'static PartsCover)>,
    joints: Query<'w, 's, &'static GlobalTransform>,
}

/// How far over the ground or the water a body is drawn before it no longer treads them: what
/// flies leaves no trail on what lies under it.
const CARRIED_OVER: f32 = 0.5;

/// How fast the gusts travel over the ground in the lightest wind, and how much faster in a gale,
/// in metres a second.
const GUST_SPEED: f32 = 1.5;
const GALE_GUST_SPEED: f32 = 2.25;

/// Every frame the covers learn the wind and who treads through them, and where each treading
/// body stood lately: only what changed of it is written. The gusts are carried on whatever wind
/// blows this frame from where the last left them, so a wind that turns or rises carries them on
/// its new way without moving them.
pub(super) fn blow(
    sources: WeatherSources,
    mut writes: ResMut<ShaderBoardWrites>,
    mut last: Local<Option<Vec<Vec4>>>,
    mut gusts: Local<Vec2>,
    mut trails: Local<HashMap<Entity, Vec<(Vec2, f32)>>>,
) {
    let WeatherSources {
        time,
        climate,
        rain,
        shelter,
        daylight,
        rig,
        ground,
        bodies,
        lying,
        joints,
    } = sources;
    let subject = rig.map_or(Vec3::ZERO, |rig| rig.target());
    // On the clock the shaders read the time by, so a trail's age is the frame's time less when
    // it was laid.
    let now = time.elapsed_secs_wrapped();
    let mut nearest: Vec<(f32, Entity, &GlobalTransform, Option<&Body>)> = bodies
        .iter()
        .filter(|(_, _, body, _)| !body.limbs().is_empty())
        .map(|(entity, placed, body, afloat)| {
            (
                placed.translation().distance_squared(subject),
                entity,
                placed,
                (!afloat).then_some(body),
            )
        })
        .collect();
    nearest.sort_by(|a, b| a.0.total_cmp(&b.0));
    nearest.truncate(TREADS);
    trails.retain(|entity, _| nearest.iter().any(|(_, near, _, _)| near == entity));
    let mut lain: Vec<(f32, &GlobalTransform, &PartsCover)> = lying
        .iter()
        .map(|(placed, parts)| {
            (
                placed.translation().distance_squared(subject),
                placed,
                parts,
            )
        })
        .collect();
    lain.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut weather = WeatherBoard {
        treads: nearest
            .into_iter()
            .map(|(_, entity, placed, body)| {
                let trail = trails.entry(entity).or_default();
                let stands = placed.translation();
                let carried_over = ground.as_ref().is_some_and(|ground| {
                    stands.y > ground.surface_at(ground_pos(stands)).0 + CARRIED_OVER
                });
                if !carried_over {
                    lay_trail(trail, stands.xz(), now);
                }
                let trod = match body {
                    Some(body) => tread(placed, body, &joints),
                    None => borne(placed),
                };
                Tread {
                    trail: trail.clone(),
                    ..trod
                }
            })
            .chain(
                lain.into_iter()
                    .map(|(_, placed, parts)| parted(placed, *parts)),
            )
            .take(TREADS)
            .collect(),
        ..WeatherBoard::default()
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
    let strength = weather.wind.length();
    *gusts += weather.wind.normalize_or_zero()
        * (GUST_SPEED + GALE_GUST_SPEED * strength)
        * time.delta_secs();
    weather.gusts = *gusts;
    let texels = weather.texels(&rain, &shelter);
    let was = last.get_or_insert_with(WeatherBoard::still);
    if let Some(changed) = ShaderBoardWrites::between(was, &texels) {
        *writes = changed;
    }
    *was = texels;
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

/// A body afloat where it is `placed`: the water bears it, so no limb of it breaks the surface,
/// and all it leaves is its trail.
fn borne(placed: &GlobalTransform) -> Tread {
    Tread {
        ground: placed.translation().xz(),
        reach: 0.0,
        limbs: Vec::new(),
        trail: Vec::new(),
    }
}

/// The cover parted round a thing lying where it is `placed`, which leaves no trail.
fn parted(placed: &GlobalTransform, parts: PartsCover) -> Tread {
    let foot = placed.translation();
    Tread {
        ground: foot.xz(),
        reach: parts.girth,
        limbs: vec![TreadingLimb {
            from: foot,
            to: foot + Vec3::Y * parts.height,
            girth: parts.girth,
        }],
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

/// How many treading bodies a cover bends round at once: those nearest the camera's subject.
pub const TREADS: usize = 24;

/// How many limbs of a treading body a cover bends round: its lowest, which reach into it first.
pub const LIMBS_PER_TREAD: usize = 32;

/// How many of the places a treading body last stood a cover reads, the newest first, and how
/// long apart they are laid: the trail a body leaves through water for its wake.
pub const TRAIL_POINTS: usize = 12;
pub const TRAIL_EVERY: f32 = 0.25;

/// The weather as the headers' `weather` reads it: the wind as a direction scaled by its
/// strength, how far its gusts have travelled over the ground on it, how far through its day the
/// game is, how clouded the sky, how much water the cloud holds, how hard it rains under the
/// wettest and how brightly lightning lights the world, and the bodies treading through.
#[derive(Clone, Debug, PartialEq)]
struct WeatherBoard {
    wind: Vec2,
    gusts: Vec2,
    daylight: f32,
    clouds: f32,
    water: f32,
    rain: f32,
    flash: f32,
    /// At most [`TREADS`].
    treads: Vec<Tread>,
}

/// A body treading through the cover: the ground point it stands on (x east, y south), how far
/// from it its limbs reach over the ground, and its limbs, at most [`LIMBS_PER_TREAD`].
#[derive(Clone, Debug, PartialEq)]
struct Tread {
    ground: Vec2,
    reach: f32,
    limbs: Vec<TreadingLimb>,
    /// Where it stood lately and when, the newest first, at most [`TRAIL_POINTS`].
    trail: Vec<(Vec2, f32)>,
}

/// A limb as it stands in the world: its bone from `from` to `to`, and how far round it its flesh
/// lies, in metres.
#[derive(Clone, Copy, Debug, PartialEq)]
struct TreadingLimb {
    from: Vec3,
    to: Vec3,
    girth: f32,
}

impl Default for WeatherBoard {
    fn default() -> WeatherBoard {
        WeatherBoard {
            wind: Vec2::ZERO,
            gusts: Vec2::ZERO,
            daylight: 0.5,
            clouds: 0.0,
            water: 0.0,
            rain: 0.0,
            flash: 0.0,
            treads: Vec::new(),
        }
    }
}

/// Where the first tread's texel lies: after the climate's, the storm's, where the rain field and
/// the shelter lie, the most the field holds, and the gusts'.
const FIRST_TREAD: usize = 7;
const FIRST_LIMB: usize = FIRST_TREAD + TREADS;
const FIRST_TRAIL: usize = FIRST_LIMB + TREADS * LIMBS_PER_TREAD * 2;
const FIRST_RAIN: usize = FIRST_TRAIL + TREADS * TRAIL_POINTS;
const FIRST_SHELTER: usize = FIRST_RAIN + RAIN_SIDE * RAIN_SIDE;
/// Texels the board holds: the climate's two, the storm's, where the rain field and the shelter
/// lie, the field's most, the gusts', a texel a tread, two a limb, one a point of a tread's trail,
/// one a cell of the rain field and one every four of the shelter's, filling out its last row.
const WEATHER_TEXELS: usize =
    (FIRST_SHELTER + SHELTER_SIDE * SHELTER_SIDE / 4).div_ceil(BOARD_WIDTH) * BOARD_WIDTH;

impl WeatherBoard {
    /// The texels of the board as it is first made, before any weather is known.
    fn still() -> Vec<Vec4> {
        WeatherBoard::default().texels(&RainField::default(), &Shelter::default())
    }

    /// The weather laid out as the headers read it, texel by texel, row by row: the wind and the
    /// count of treads in the first texel, the daylight, the cloud, its water and the rain in the
    /// second; the flash and where the rain field's, the shelter's and the treads' first texels
    /// lie in the third; where the rain field and the shelter lie on the ground in the fourth and
    /// fifth, their north-west corner, a cell's side and the cells each way, or nothing for one
    /// not held; the wettest ground and the hardest rain over the field in the sixth, so a shader
    /// skips reading the field where it lies dry, and how far the rain leans as it falls; how far
    /// the gusts have travelled in the seventh; then a texel a tread, its ground point, its reach
    /// and its count of limbs, and after every tread's, [`LIMBS_PER_TREAD`] places of two texels
    /// for each tread's limbs, a bone's start and girth, then its end, and after those
    /// [`TRAIL_POINTS`] texels for each tread's trail, a point's place, when it was laid and 1, or
    /// nothing past the trail's end; then a texel a cell of the rain field, its share of the
    /// downpour, its wetness and its rain, and the tops of the shelter's cells, four to a texel.
    fn texels(&self, rain: &RainField, shelter: &Shelter) -> Vec<Vec4> {
        let mut texels = vec![Vec4::ZERO; WEATHER_TEXELS];
        texels[0] = Vec4::new(self.wind.x, self.wind.y, 0.0, self.treads.len() as f32);
        texels[1] = Vec4::new(self.daylight, self.clouds, self.water, self.rain);
        texels[2] = Vec4::new(
            self.flash,
            FIRST_RAIN as f32,
            FIRST_SHELTER as f32,
            FIRST_TREAD as f32,
        );
        if rain.cells.len() == RAIN_SIDE * RAIN_SIDE {
            texels[3] = rain.corner.extend(RAIN_CELL.0).extend(RAIN_SIDE as f32);
            texels[5] = rain.cells.iter().fold(Vec4::ZERO, |most, cell| {
                most.max(Vec4::new(cell.wetness, cell.rain, 0.0, 0.0))
            });
            for (texel, cell) in texels[FIRST_RAIN..].iter_mut().zip(&rain.cells) {
                *texel = Vec4::new(cell.share, cell.wetness, cell.rain, 0.0);
            }
        }
        texels[5].z = shelter.lean.x;
        texels[5].w = shelter.lean.y;
        texels[6] = self.gusts.extend(0.0).extend(0.0);
        if shelter.tops.len() == SHELTER_SIDE * SHELTER_SIDE {
            texels[4] = shelter
                .corner
                .extend(SHELTER_CELL.0)
                .extend(SHELTER_SIDE as f32);
            let (fours, _) = shelter.tops.as_chunks::<4>();
            for (texel, four) in texels[FIRST_SHELTER..].iter_mut().zip(fours) {
                *texel = Vec4::from_array(*four);
            }
        }
        for (index, tread) in self.treads.iter().take(TREADS).enumerate() {
            let limbs = &tread.limbs[..tread.limbs.len().min(LIMBS_PER_TREAD)];
            texels[FIRST_TREAD + index] =
                tread.ground.extend(tread.reach).extend(limbs.len() as f32);
            let first = FIRST_LIMB + index * LIMBS_PER_TREAD * 2;
            for (slot, limb) in limbs.iter().enumerate() {
                texels[first + slot * 2] = limb.from.extend(limb.girth);
                texels[first + slot * 2 + 1] = limb.to.extend(0.0);
            }
            let trail = FIRST_TRAIL + index * TRAIL_POINTS;
            for (slot, (at, laid)) in tread.trail.iter().take(TRAIL_POINTS).enumerate() {
                texels[trail + slot] = at.extend(*laid).extend(1.0);
            }
        }
        texels
    }
}
