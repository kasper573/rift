//! The weather the world's shaders read: the shown area's weather, the rain over the ground round
//! the camera's subject and the bodies treading through the cover, published each frame on the
//! board they read. Every body treads through the cover on its limbs, as its skeleton moves them:
//! a cover bends round the lowest limbs of the bodies nearest the camera's subject.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::shader::Shader;

use super::heavens::{RainMaterial, SkyMaterial};
use super::rain::{RAIN_CELL, RAIN_SIDE, RainField, SHELTER_CELL, SHELTER_SIDE, Shelter};
use super::stages;
use crate::core::interpolate::Jumps;
use crate::core::render::world::afloat::{Aboard, Afloat};
use crate::core::render::world::body::Body;
use crate::core::render::world::gpu::draw::{GROUND_LIT_SHADER, ground_lit_shader};
use crate::core::render::world::sky::Daylight;
use crate::core::render::{CameraRig, GroundHeights, ground_pos};
use crate::core::vfx::board::{BOARD_WIDTH, ShaderBoardPlugin, ShaderBoardWrites};
use crate::systems::mount::render::Steed;
use crate::systems::movement::Position;
use crate::systems::water::ripples::{
    RIPPLE_CELL, RIPPLE_SIDE, RippleField, RippleSource, RipplesPlugin,
};

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
        .init_resource::<Treading>()
        .add_systems(Update, (tread, blow).chain())
        .add_plugins(RipplesPlugin);
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
    /// How its rain breaks the water it strikes, none where it leaves the water as it is.
    pub ripples: Option<RippleSource>,
}

/// A thing lying on the ground that the cover parts round as round a body's limb: `girth` round
/// where it stands, from the ground up to `height`. Bodies tread first, so among many a thing
/// may lie in standing cover.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct PartsCover {
    pub girth: f32,
    pub height: f32,
}

/// The bodies treading nearest the camera's subject this frame, the nearest first, at most
/// [`TREADS`].
#[derive(Resource, Default)]
pub struct Treading(pub Vec<TreadingBody>);

/// A body as it treads: the ground point it stands on (x east, y south), how far from it its
/// limbs reach over the ground, its lowest limbs where its joints hold them, at most
/// [`LIMBS_PER_TREAD`], the level of the water it stands or floats in, none on dry ground, and
/// the count of the times it was put where it is rather than moved there.
#[derive(Clone, Debug, PartialEq)]
pub struct TreadingBody {
    pub entity: Entity,
    pub ground: Vec2,
    pub reach: f32,
    pub limbs: Vec<TreadingLimb>,
    pub water: Option<f32>,
    pub put: Option<Jumps>,
}

/// A limb as it stands in the world: its bone from `from` to `to`, and how far round it its flesh
/// lies at each end, in metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TreadingLimb {
    pub from: Vec3,
    pub to: Vec3,
    pub girth: f32,
    pub tail_girth: f32,
}

/// Who treads the ground, and where the camera looks.
#[derive(SystemParam)]
pub(crate) struct TreadSources<'w, 's> {
    rig: Option<Res<'w, CameraRig>>,
    ground: Option<Res<'w, GroundHeights>>,
    bodies: Query<
        'w,
        's,
        (
            Entity,
            &'static GlobalTransform,
            &'static Body,
            Option<&'static Afloat>,
        ),
        Without<Aboard>,
    >,
    joints: Query<'w, 's, &'static GlobalTransform>,
    positions: Query<'w, 's, &'static Position>,
    steeds: Query<'w, 's, &'static Steed>,
}

/// Every frame, tells which bodies tread nearest the camera's subject, each as its skeleton
/// holds its limbs.
pub(crate) fn tread(sources: TreadSources, mut treading: ResMut<Treading>) {
    let TreadSources {
        rig,
        ground,
        bodies,
        joints,
        positions,
        steeds,
    } = sources;
    let subject = rig.map_or(Vec3::ZERO, |rig| rig.target());
    let mut nearest: Vec<(f32, Entity, &GlobalTransform, &Body, Option<&Afloat>)> = bodies
        .iter()
        .filter(|(_, _, body, _)| !body.limbs().is_empty())
        .map(|(entity, placed, body, afloat)| {
            (
                placed.translation().distance_squared(subject),
                entity,
                placed,
                body,
                afloat,
            )
        })
        .collect();
    nearest.sort_by(|a, b| a.0.total_cmp(&b.0));
    treading.0 = nearest
        .into_iter()
        .take(TREADS)
        .map(|(_, entity, placed, body, afloat)| {
            let stands = placed.translation();
            let limbs = limbs_of(body, &joints);
            TreadingBody {
                entity,
                ground: stands.xz(),
                reach: reach(stands.xz(), &limbs),
                limbs,
                water: afloat.map(|afloat| afloat.level.0).or_else(|| {
                    let ground = ground.as_ref()?;
                    let at = ground_pos(stands);
                    (ground.water.depth_at(&ground.heights, at).0 > 0.0)
                        .then(|| ground.water.height_at(at).0)
                }),
                put: positions
                    .get(steeds.get(entity).map_or(entity, |steed| steed.rider))
                    .ok()
                    .map(Position::put),
            }
        })
        .collect();
}

/// What the weather is made of: the moment, the shown area's weather, its rain and daylight, who
/// treads the ground and what lies on it, and where the field of the water's ripples lies.
#[derive(SystemParam)]
pub(crate) struct WeatherSources<'w, 's> {
    time: Res<'w, Time>,
    climate: Option<Res<'w, ShownWeather>>,
    rain: Res<'w, RainField>,
    shelter: Res<'w, Shelter>,
    daylight: Option<Res<'w, Daylight>>,
    rig: Option<Res<'w, CameraRig>>,
    treading: Res<'w, Treading>,
    lying: Query<'w, 's, (&'static GlobalTransform, &'static PartsCover)>,
    ripples: Res<'w, RippleField>,
}

/// How fast the gusts travel over the ground in the lightest wind, and how much faster in a gale,
/// in metres a second.
const GUST_SPEED: f32 = 1.5;
const GALE_GUST_SPEED: f32 = 2.25;

/// Every frame the covers learn the wind and who treads through them: only what changed of it is
/// written. The gusts are carried on whatever wind blows this frame from where the last left
/// them, so a wind that turns or rises carries them on its new way without moving them.
pub(crate) fn blow(
    sources: WeatherSources,
    mut writes: ResMut<ShaderBoardWrites>,
    mut last: Local<Option<Vec<Vec4>>>,
    mut gusts: Local<Vec2>,
) {
    let WeatherSources {
        time,
        climate,
        rain,
        shelter,
        daylight,
        rig,
        treading,
        lying,
        ripples,
    } = sources;
    let subject = rig.map_or(Vec3::ZERO, |rig| rig.target());
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
        treads: treading
            .0
            .iter()
            .map(|body| Tread {
                ground: body.ground,
                reach: body.reach,
                limbs: body.limbs.clone(),
            })
            .chain(
                lain.into_iter()
                    .map(|(_, placed, parts)| parted(placed, *parts)),
            )
            .take(TREADS)
            .collect(),
        rippling: ripples.corner(),
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

/// The lowest limbs of `body` where its joints hold them now, the lowest first.
fn limbs_of(body: &Body, joints: &Query<&GlobalTransform>) -> Vec<TreadingLimb> {
    let mut limbs: Vec<TreadingLimb> = body
        .limbs()
        .iter()
        .filter_map(|limb| {
            let joint = joints.get(limb.joint).ok()?;
            let scale = joint.scale().abs().max_element();
            Some(TreadingLimb {
                from: joint.transform_point(limb.head),
                to: joint.transform_point(limb.tail),
                girth: limb.girth * scale,
                tail_girth: limb.tail_girth * scale,
            })
        })
        .collect();
    limbs.sort_by(|a, b| lowest(a).total_cmp(&lowest(b)));
    limbs.truncate(LIMBS_PER_TREAD);
    limbs
}

/// How far from `ground` the `limbs` reach over the ground.
fn reach(ground: Vec2, limbs: &[TreadingLimb]) -> f32 {
    limbs
        .iter()
        .map(|limb| {
            ground
                .distance(limb.from.xz())
                .max(ground.distance(limb.to.xz()))
                + limb.girth.max(limb.tail_girth)
        })
        .fold(0.0, f32::max)
}

/// The cover parted round a thing lying where it is `placed`.
fn parted(placed: &GlobalTransform, parts: PartsCover) -> Tread {
    let foot = placed.translation();
    Tread {
        ground: foot.xz(),
        reach: parts.girth,
        limbs: vec![TreadingLimb {
            from: foot,
            to: foot + Vec3::Y * parts.height,
            girth: parts.girth,
            tail_girth: parts.girth,
        }],
    }
}

fn lowest(limb: &TreadingLimb) -> f32 {
    (limb.from.y - limb.girth).min(limb.to.y - limb.tail_girth)
}

/// How many treading bodies a cover bends round at once: those nearest the camera's subject.
pub const TREADS: usize = 24;

/// How many limbs of a treading body a cover bends round: its lowest, which reach into it first.
pub const LIMBS_PER_TREAD: usize = 32;

/// The weather as the headers' `weather` reads it: the wind as a direction scaled by its
/// strength, how far its gusts have travelled over the ground on it, how far through its day the
/// game is, how clouded the sky, how much water the cloud holds, how hard it rains under the
/// wettest and how brightly lightning lights the world, the bodies treading through, and the
/// ground point of the north-west corner of the field of the water's ripples.
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
    rippling: Vec2,
}

/// What a cover bends round: the ground point it stands on (x east, y south), how far from it
/// its limbs reach over the ground, and its limbs, at most [`LIMBS_PER_TREAD`].
#[derive(Clone, Debug, PartialEq)]
struct Tread {
    ground: Vec2,
    reach: f32,
    limbs: Vec<TreadingLimb>,
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
            rippling: Vec2::ZERO,
        }
    }
}

/// The texel that tells where the field of the water's ripples lies.
pub const RIPPLE_WINDOW: usize = 7;
/// Where the first tread's texel lies: after the climate's, the storm's, where the rain field and
/// the shelter lie, the most the field holds, the gusts', and where the ripples lie.
const FIRST_TREAD: usize = RIPPLE_WINDOW + 1;
const FIRST_LIMB: usize = FIRST_TREAD + TREADS;
const FIRST_RAIN: usize = FIRST_LIMB + TREADS * LIMBS_PER_TREAD * 2;
const FIRST_SHELTER: usize = FIRST_RAIN + RAIN_SIDE * RAIN_SIDE;
/// Texels the board holds: the climate's two, the storm's, where the rain field and the shelter
/// lie, the field's most, the gusts', where the ripples lie, a texel a tread, two a limb, one a
/// cell of the rain field and one every four of the shelter's, filling out its last row.
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
    /// the gusts have travelled in the seventh; where the field of the water's ripples lies in
    /// the eighth, as the rain field's is told; then a texel a tread, its ground point, its reach
    /// and its count of limbs, and after every tread's, [`LIMBS_PER_TREAD`] places of two texels
    /// for each tread's limbs, a bone's start and girth, then its end and girth; then a texel a
    /// cell of the rain field, its share of the downpour, its wetness and its rain, and the tops
    /// of the shelter's cells, four to a texel.
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
        texels[RIPPLE_WINDOW] = self
            .rippling
            .extend(RIPPLE_CELL.0)
            .extend(RIPPLE_SIDE as f32);
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
                texels[first + slot * 2 + 1] = limb.to.extend(limb.tail_girth);
            }
        }
        texels
    }
}
