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
use crate::core::content::Content;
use crate::core::interpolate::Jumps;
use crate::core::render::world::afloat::{Aboard, Afloat};
use crate::core::render::world::body::Body;
use crate::core::render::world::gpu::draw::{GROUND_LIT_SHADER, ground_lit_shader};
use crate::core::render::world::sky::Daylight;
use crate::core::render::{CameraRig, GroundHeights, ground_pos};
use crate::core::vfx::board::{BOARD_WIDTH, ShaderBoardPlugin, ShaderBoardWrites};
use crate::systems::GameSettings;
use crate::systems::mount::render::Steed;
use crate::systems::movement::Position;
use crate::systems::water::ripples::{RippleSpot, Ripples};

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
    /// The rings its rain sends over water where it strikes, none where it sends none.
    pub ripples: Option<Ripples>,
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
    lying: Query<'w, 's, (&'static GlobalTransform, &'static PartsCover)>,
    joints: Query<'w, 's, &'static GlobalTransform>,
    positions: Query<'w, 's, &'static Position>,
    steeds: Query<'w, 's, &'static Steed>,
    spots: Query<'w, 's, (&'static GlobalTransform, &'static RippleSpot)>,
    content: Option<Res<'w, Content>>,
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
    mut wading: Local<HashMap<Entity, Wading>>,
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
        positions,
        steeds,
        spots,
        content,
    } = sources;
    let subject = rig.map_or(Vec3::ZERO, |rig| rig.target());
    // On the clock the shaders read the time by, so a trail's age is the frame's time less when
    // it was laid.
    let now = time.elapsed_secs_wrapped();
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
    nearest.truncate(TREADS);
    wading.retain(|entity, _| nearest.iter().any(|(_, near, _, _, _)| near == entity));
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
    let mut rippling: Vec<(f32, Vec3, Ripples)> = spots
        .iter()
        .map(|(placed, spot)| {
            let at = placed.translation();
            (at.distance_squared(subject), at, spot.0)
        })
        .collect();
    rippling.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut weather = WeatherBoard {
        treads: nearest
            .into_iter()
            .map(|(_, entity, placed, body, afloat)| {
                let waded = wading.entry(entity).or_default();
                let put = positions
                    .get(steeds.get(entity).map_or(entity, |steed| steed.rider))
                    .ok()
                    .map(Position::put);
                if std::mem::replace(&mut waded.put, put) != put {
                    waded.trail.clear();
                }
                let stands = placed.translation();
                let carried_over = ground.as_ref().is_some_and(|ground| {
                    stands.y > ground.surface_at(ground_pos(stands)).0 + CARRIED_OVER
                });
                let heading = waded.trail.first().map(|(stood, _)| stands.xz() - *stood);
                let speed = waded.trail.first().map_or(0.0, |(stood, laid)| {
                    stands.xz().distance(*stood) / (now - laid).max(time.delta_secs())
                });
                if !carried_over {
                    lay_trail(&mut waded.trail, stands.xz(), now);
                }
                let trod = tread(placed, body, &joints);
                let level = afloat.map(|afloat| afloat.level.0).or_else(|| {
                    let ground = ground.as_ref()?;
                    let at = ground_pos(stands);
                    (ground.water.depth_at(&ground.heights, at).0 > 0.0)
                        .then(|| ground.water.height_at(at).0)
                });
                let ahead = (placed.rotation() * Vec3::Z).xz().normalize_or(Vec2::Y);
                let breaking: Vec<(Vec2, f32)> = level.map_or_else(Vec::new, |level| {
                    breaking(&trod.limbs, level)
                        .into_iter()
                        .map(|(at, radius)| (framed(ahead, at - trod.ground), radius))
                        .collect()
                });
                settle(&mut waded.wetted, &breaking, time.delta_secs());
                Tread {
                    wetted: leading(&waded.wetted, heading.map(|way| framed(ahead, way)))
                        .into_iter()
                        .map(|place| Wetted {
                            off: ahead * place.off.x + ahead.perp() * place.off.y,
                            ..place
                        })
                        .collect(),
                    draught: level.map_or(0.0, |level| {
                        (level - trod.limbs.iter().map(lowest).fold(f32::MAX, f32::min)).max(0.0)
                    }),
                    speed,
                    trail: waded.trail.clone(),
                    ..trod
                }
            })
            .chain(
                lain.into_iter()
                    .map(|(_, placed, parts)| parted(placed, *parts)),
            )
            .take(TREADS)
            .collect(),
        spots: rippling
            .into_iter()
            .take(RIPPLE_SPOTS)
            .map(|(_, at, ripples)| (at, ripples))
            .collect(),
        resting: content.and_then(|content| content.built::<GameSettings>().ripples),
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
        weather.raining = climate.ripples;
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
    let ground = placed.translation().xz();
    let reach = limbs
        .iter()
        .map(|limb| {
            ground
                .distance(limb.from.xz())
                .max(ground.distance(limb.to.xz()))
                + limb.girth.max(limb.tail_girth)
        })
        .fold(0.0, f32::max);
    Tread {
        ground,
        reach,
        limbs,
        wetted: Vec::new(),
        draught: 0.0,
        speed: 0.0,
        trail: Vec::new(),
    }
}

/// Where the `limbs` of a body break the water lying at `level`, and how far round each place
/// its flesh lies in the surface: a limb standing through the surface breaks it once, one lying in
/// it (a hull) at both ends of what lies there. Places within each other's flesh count once, as
/// the larger.
fn breaking(limbs: &[TreadingLimb], level: f32) -> Vec<(Vec2, f32)> {
    let mut places: Vec<(Vec2, f32)> = Vec::new();
    let mut lay = |(at, girth): (Vec3, f32)| {
        let radius = (girth * girth - (at.y - level) * (at.y - level))
            .max(0.0)
            .sqrt();
        if radius <= 0.0 {
            return;
        }
        match places
            .iter_mut()
            .find(|laid| laid.0.distance(at.xz()) < laid.1.max(radius))
        {
            Some(laid) if laid.1 < radius => *laid = (at.xz(), radius),
            Some(_) => {}
            None => places.push((at.xz(), radius)),
        }
    };
    for limb in limbs {
        let Some((from, to)) = limb.in_surface(level) else {
            continue;
        };
        let (head, tail) = (limb.at(from), limb.at(to));
        if head.0.xz().distance(tail.0.xz()) <= head.1.max(tail.1) {
            lay(limb.at(limb.nearest_level(level).clamp(from, to)));
        } else {
            lay(head);
            lay(tail);
        }
    }
    places
}

/// Brings the places a body is remembered to break the water, in its own frame, toward where it
/// breaks it now, over `passed` seconds. A wake is left by where a limb breaks the water over a
/// while, not by where it is this frame: a leg swings as it wades and a foot leaves the water at
/// every step, and the wake behind them does neither. So a remembered place follows the nearest
/// of the fresh ones and grows steady while one answers it, and one that none answers fades and
/// is forgotten.
fn settle(wetted: &mut Vec<Wetted>, breaking: &[(Vec2, f32)], passed: f32) {
    let share = 1.0 - (-passed / SETTLES_IN).exp();
    let mut answered = vec![false; wetted.len()];
    for (off, radius) in breaking {
        let nearest = wetted
            .iter()
            .enumerate()
            .filter(|(slot, _)| !answered[*slot])
            .map(|(slot, place)| (slot, place.off.distance(*off)))
            .filter(|(_, apart)| *apart < SAME_PLACE)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        match nearest {
            Some((slot, _)) => {
                answered[slot] = true;
                let place = &mut wetted[slot];
                place.off = place.off.lerp(*off, share);
                place.radius += (radius - place.radius) * share;
                place.steady += (1.0 - place.steady) * share;
            }
            None => {
                answered.push(true);
                wetted.push(Wetted {
                    off: *off,
                    radius: *radius,
                    steady: share,
                });
            }
        }
    }
    for (place, answered) in wetted.iter_mut().zip(answered) {
        if !answered {
            place.steady -= place.steady * share;
        }
    }
    wetted.retain(|place| place.steady > FORGOTTEN);
}

/// The places of `wetted` a water reads, at most [`WETTED_PER_TREAD`]: of more, the foremost and
/// the hindmost along the body's `heading` first, which lead and close its wake, then the
/// largest.
fn leading(wetted: &[Wetted], heading: Option<Vec2>) -> Vec<Wetted> {
    let mut places = wetted.to_vec();
    if places.len() <= WETTED_PER_TREAD {
        return places;
    }
    let way = heading
        .map(Vec2::normalize_or_zero)
        .filter(|way| *way != Vec2::ZERO);
    if let Some(way) = way {
        places.sort_by(|a, b| b.off.dot(way).total_cmp(&a.off.dot(way)));
        let hindmost = places.pop();
        places[1..].sort_by(|a, b| b.radius.total_cmp(&a.radius));
        places.splice(1..1, hindmost);
    } else {
        places.sort_by(|a, b| b.radius.total_cmp(&a.radius));
    }
    places.truncate(WETTED_PER_TREAD);
    places
}

/// `offset`, which lies on the ground, in the frame of a body facing `ahead`: how far ahead of
/// it in x, and how far to its side in y.
fn framed(ahead: Vec2, offset: Vec2) -> Vec2 {
    Vec2::new(offset.dot(ahead), offset.dot(ahead.perp()))
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
            tail_girth: parts.girth,
        }],
        wetted: Vec::new(),
        draught: 0.0,
        speed: 0.0,
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
    (limb.from.y - limb.girth).min(limb.to.y - limb.tail_girth)
}

/// How many treading bodies a cover bends round at once: those nearest the camera's subject.
pub const TREADS: usize = 24;

/// How many limbs of a treading body a cover bends round: its lowest, which reach into it first.
pub const LIMBS_PER_TREAD: usize = 32;

/// How many of the places a treading body last stood a cover reads, the newest first, and how
/// long apart they are laid: the trail a body leaves through water for its wake.
pub const TRAIL_POINTS: usize = 12;
const TRAIL_EVERY: f32 = 0.3;

/// How many of the places a treading body breaks the water a water reads: each the head of a
/// wake.
pub const WETTED_PER_TREAD: usize = 6;

/// How long a place a body breaks the water takes to settle where the body breaks it now, in
/// seconds: longer than a step, so a wader's wake keeps to the middle of each leg's swing.
const SETTLES_IN: f32 = 0.7;
/// How near each other, in metres, a remembered place and a fresh one are the same place, and
/// how unsteady a remembered one is forgotten.
const SAME_PLACE: f32 = 0.5;
const FORGOTTEN: f32 = 0.02;

/// How many things that keep the water round them broken a water reads at once: those nearest
/// the camera's subject.
pub const RIPPLE_SPOTS: usize = 16;

/// How much a wake widens for every metre it trails behind what leaves it: its edge runs back at
/// the angle whose sine is a third whatever the speed, as a ship's does in deep water.
pub const WAKE_SPREAD: f32 = 1.0 / 3.0;

/// The weather as the headers' `weather` reads it: the wind as a direction scaled by its
/// strength, how far its gusts have travelled over the ground on it, how far through its day the
/// game is, how clouded the sky, how much water the cloud holds, how hard it rains under the
/// wettest and how brightly lightning lights the world, the bodies treading through, and the
/// rings that spread over water: those a body at rest sends, those the rain does, and where each
/// thing that keeps the water round it broken stands, with the rings it sends.
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
    resting: Option<Ripples>,
    raining: Option<Ripples>,
    /// At most [`RIPPLE_SPOTS`].
    spots: Vec<(Vec3, Ripples)>,
}

/// A body treading through the cover: the ground point it stands on (x east, y south), how far
/// from it its limbs reach over the ground, and its limbs, at most [`LIMBS_PER_TREAD`].
#[derive(Clone, Debug, PartialEq)]
struct Tread {
    ground: Vec2,
    reach: f32,
    limbs: Vec<TreadingLimb>,
    /// Where it breaks the water it stands or floats in, at most [`WETTED_PER_TREAD`], each
    /// offset from its ground point.
    wetted: Vec<Wetted>,
    /// How deep it reaches under that water's surface, in metres.
    draught: f32,
    /// How fast it travels over the ground, in metres a second.
    speed: f32,
    /// Where it stood lately and when, the newest first, at most [`TRAIL_POINTS`].
    trail: Vec<(Vec2, f32)>,
}

/// A limb as it stands in the world: its bone from `from` to `to`, and how far round it its flesh
/// lies at each end, in metres.
#[derive(Clone, Copy, Debug, PartialEq)]
struct TreadingLimb {
    from: Vec3,
    to: Vec3,
    girth: f32,
    tail_girth: f32,
}

impl Tread {
    /// How far from where the body stands its wake reaches at most: as far as its trail runs,
    /// and as wide round the trail's end as [`WAKE_SPREAD`] opens it from the place that breaks
    /// the water farthest out.
    fn wake_reach(&self) -> f32 {
        let trailed: f32 = std::iter::once(self.ground)
            .chain(self.trail.iter().map(|(stood, _)| *stood))
            .collect::<Vec<Vec2>>()
            .windows(2)
            .map(|stretch| stretch[0].distance(stretch[1]))
            .sum();
        let widest = self
            .wetted
            .iter()
            .map(|place| place.off.length() + place.radius)
            .fold(0.0, f32::max);
        trailed * (1.0 + WAKE_SPREAD) + widest
    }
}

impl TreadingLimb {
    /// The point `along` the bone, 0 its start and 1 its end, and its girth there.
    fn at(&self, along: f32) -> (Vec3, f32) {
        (
            self.from.lerp(self.to, along),
            self.girth + (self.tail_girth - self.girth) * along,
        )
    }

    /// How far along the bone it comes nearest a surface at `level`.
    fn nearest_level(&self, level: f32) -> f32 {
        let rise = self.to.y - self.from.y;
        if rise.abs() < 1e-5 {
            return 0.5;
        }
        ((level - self.from.y) / rise).clamp(0.0, 1.0)
    }

    /// The stretch of the bone whose flesh a surface at `level` cuts through, from where along
    /// it to where; none for a limb wholly over or under the surface.
    fn in_surface(&self, level: f32) -> Option<(f32, f32)> {
        let girth = self.girth.max(self.tail_girth);
        let (from, to) = (self.from.y - level, self.to.y - level);
        let rise = to - from;
        if rise.abs() < 1e-5 {
            return (from.abs() < girth).then_some((0.0, 1.0));
        }
        let (a, b) = ((-girth - from) / rise, (girth - from) / rise);
        let (first, last) = (a.min(b).max(0.0), a.max(b).min(1.0));
        (first < last).then_some((first, last))
    }
}

/// A place a treading body breaks the water: how far from the ground point it stands on, and how
/// far round that place its flesh lies in the surface, in metres, and how steadily it has broken
/// the surface there of late, from 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Wetted {
    off: Vec2,
    radius: f32,
    steady: f32,
}

/// What a water remembers of a body treading it: where it stood lately and when, the newest
/// first, and the places it breaks the surface, in its own frame. A body put somewhere rather
/// than moved there, as `put` counts, came no way to trail a wake along.
#[derive(Default)]
pub(super) struct Wading {
    trail: Vec<(Vec2, f32)>,
    wetted: Vec<Wetted>,
    put: Option<Jumps>,
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
            resting: None,
            raining: None,
            spots: Vec::new(),
        }
    }
}

/// Where the first tread's texel lies: after the climate's, the storm's, where the rain field and
/// the shelter lie, the most the field holds, and the gusts'.
const FIRST_TREAD: usize = 7;
const FIRST_LIMB: usize = FIRST_TREAD + TREADS;
const FIRST_TRAIL: usize = FIRST_LIMB + TREADS * LIMBS_PER_TREAD * 2;
const FIRST_WAKE: usize = FIRST_TRAIL + TREADS * TRAIL_POINTS;
const FIRST_WETTED: usize = FIRST_WAKE + TREADS;
const FIRST_RIPPLES: usize = FIRST_WETTED + TREADS * WETTED_PER_TREAD;
const FIRST_SPOT: usize = FIRST_RIPPLES + 3;
const FIRST_RAIN: usize = FIRST_SPOT + RIPPLE_SPOTS * 2;
const FIRST_SHELTER: usize = FIRST_RAIN + RAIN_SIDE * RAIN_SIDE;
/// Texels the board holds: the climate's two, the storm's, where the rain field and the shelter
/// lie, the field's most, the gusts', a texel a tread, two a limb, one a point of a tread's trail,
/// one for what leads its wake and one a place it breaks the water, three for the rings that
/// spread over water and two a spot they spread from, one a cell of the rain field and one every
/// four of the shelter's, filling out its last row.
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
    /// for each tread's limbs, a bone's start and girth, then its end and girth, after those
    /// [`TRAIL_POINTS`] texels for each tread's trail, a point's place, when it was laid and 1, or
    /// nothing past the trail's end, a texel for what leads each tread's wake, how far from the
    /// tread it reaches, how many places the tread breaks the water, how fast it travels and how
    /// deep it reaches, and [`WETTED_PER_TREAD`] for those places, each its offset from the
    /// tread's ground point, how far round it the flesh lies and how steady it is, or nothing;
    /// then how many spots send rings with how strong those of a body at rest and of the rain
    /// are drawn, the rings of a body at rest and the rain's as [`Ripples::told`] tells them,
    /// and two texels a spot, where it stands with how strong its rings are drawn, then its
    /// rings; then a texel a cell of the rain field, its share of the downpour, its wetness and its
    /// rain, and the tops of the shelter's cells, four to a texel.
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
                texels[first + slot * 2 + 1] = limb.to.extend(limb.tail_girth);
            }
            let trail = FIRST_TRAIL + index * TRAIL_POINTS;
            for (slot, (at, laid)) in tread.trail.iter().take(TRAIL_POINTS).enumerate() {
                texels[trail + slot] = at.extend(*laid).extend(1.0);
            }
            let places = &tread.wetted[..tread.wetted.len().min(WETTED_PER_TREAD)];
            texels[FIRST_WAKE + index] = Vec4::new(
                tread.wake_reach(),
                places.len() as f32,
                tread.speed,
                tread.draught,
            );
            let wetted = FIRST_WETTED + index * WETTED_PER_TREAD;
            for (slot, place) in places.iter().enumerate() {
                texels[wetted + slot] = place.off.extend(place.radius).extend(place.steady);
            }
        }
        let spots = &self.spots[..self.spots.len().min(RIPPLE_SPOTS)];
        let strength = |ripples: Option<Ripples>| ripples.map_or(0.0, |ripples| ripples.strength);
        texels[FIRST_RIPPLES] = Vec4::new(
            spots.len() as f32,
            strength(self.resting),
            strength(self.raining),
            0.0,
        );
        texels[FIRST_RIPPLES + 1] = self.resting.map_or(Vec4::ZERO, |ripples| ripples.told());
        texels[FIRST_RIPPLES + 2] = self.raining.map_or(Vec4::ZERO, |ripples| ripples.told());
        for (slot, (at, ripples)) in spots.iter().enumerate() {
            texels[FIRST_SPOT + slot * 2] = at.extend(ripples.strength);
            texels[FIRST_SPOT + slot * 2 + 1] = ripples.told();
        }
        texels
    }
}
