//! The ripples that travel over water from where its surface is broken: a field of how far the
//! surface stands over or under its level round the camera's subject, in which whatever disturbs
//! it spreads as waves at the game's one speed and fades. Whatever breaks the surface stirs it by
//! how its flesh moved through the surface since the frame before, limb by limb as its skeleton
//! holds them: a hull pushes a wave ahead of its bow and trails a wake as wide as it is, a wader
//! one from each leg in the shallows and one from the hips where it stands deep, a body that
//! only shifts its weight sends a ring as slight as the shift, and what stands wholly still
//! leaves the water still. Rain, and a prop that stirs the water it stands in, as a shoal of
//! fish does, break it as the [`RippleSource`] each names tells. Beside the waves the field
//! holds the froth: where the water was churned by what broke it, which stays where it was
//! churned and settles. A water's shader reads both and draws them as it likes.

use std::ops::Range;

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::{Content, RowCheck};
use crate::core::interpolate::Jumps;
use crate::core::math::Rng;
use crate::core::render::{CameraRig, GroundHeights, ground_pos};
use crate::core::units::{Meters, MetersPerSec, Seconds};
use crate::core::vfx::field::{ShaderFieldPlugin, ShaderFieldWrite};
use crate::systems::GameSettings;
use crate::systems::area::rain::{RainField, Shelter};
use crate::systems::area::shown_weather::{ShownWeather, Treading, TreadingLimb, blow, tread};

/// How ripples travel over the game's waters: at `speed`, fading to a twentieth over `lasts` on
/// calm water and over `lasts_in_gale` in a gale, whose chop swallows them sooner; how long the
/// `froth` lies where the water was churned before it has settled to a twentieth; and how fast
/// flesh moves through the water to churn it to froth all over, `froths_at`: slower flesh
/// churns it less, by the square of its speed, so a wader leaves a lace and a hull a wake.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ripples {
    pub speed: MetersPerSec,
    pub lasts: Seconds,
    pub lasts_in_gale: Seconds,
    pub froth: Seconds,
    pub froths_at: MetersPerSec,
}

/// What breaks the water without a body moving through it: once `every` so often, anywhere
/// within `scatter` of where it is, over a place `size` wide and `strength` strong, 0 to 1, a
/// full one as flesh that wide come into the water.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RippleSource {
    pub every: Seconds,
    #[serde(default, skip_serializing_if = "Meters::is_zero")]
    pub scatter: Meters,
    pub size: Meters,
    pub strength: f32,
}

/// Marks a thing standing in the world that keeps the water round it broken.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct RippleSpot(pub RippleSource);

/// How many cells of the field lie each way round the camera's subject, and how wide each is:
/// fine enough to tell a wader's legs apart.
pub const RIPPLE_SIDE: usize = 512;
pub const RIPPLE_CELL: Meters = Meters(0.08);

/// The fastest a ripple may travel: faster, a wave would cross more of a cell in a step than
/// the field can carry it.
pub const FASTEST_RIPPLE: MetersPerSec = MetersPerSec(Meters(
    RIPPLE_CELL.0 / STEP * std::f32::consts::FRAC_1_SQRT_2,
));

/// Keeps the field of the water's ripples, and the picture of it the waters' shaders read.
pub struct RipplesPlugin;

impl Plugin for RipplesPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ShaderFieldPlugin { side: RIPPLE_SIDE })
            .init_resource::<RippleField>()
            .add_systems(Update, stir.after(tread).before(blow));
    }
}

/// The water's rise and fall round the camera's subject, a cell every [`RIPPLE_CELL`], row by
/// row from its north-west corner. It lies still until something breaks the water, and again
/// once what broke it has faded.
#[derive(Resource)]
pub struct RippleField {
    /// The cell of the ground its north-west corner lies in.
    corner: IVec2,
    now: Vec<f32>,
    before: Vec<f32>,
    /// What the next step is worked out into.
    next: Vec<f32>,
    /// How churned the water is, from still at 0 to all froth at 1.
    froth: Vec<f32>,
    /// The time passed that no step has answered for yet, in seconds.
    owed: f32,
    /// How long ago the water was last broken, in seconds; none while it lies still.
    quiet: Option<f32>,
    unpublished: bool,
}

/// The flesh of a body in water: its limbs, and the level of the water they stand or float in.
#[derive(Clone, Copy, Debug)]
pub struct Wading<'a> {
    pub limbs: &'a [TreadingLimb],
    pub level: f32,
}

impl Ripples {
    pub fn check(&self, pointer: &str, cx: &mut RowCheck) {
        cx.range(
            format!("{pointer}/speed"),
            self.speed.0.0,
            0.05,
            FASTEST_RIPPLE.0.0,
            "meters a second",
        );
        for (field, lasts) in [
            ("lasts", self.lasts),
            ("lasts_in_gale", self.lasts_in_gale),
            ("froth", self.froth),
        ] {
            cx.range(format!("{pointer}/{field}"), lasts.0, 0.05, 60.0, "seconds");
        }
        cx.range(
            format!("{pointer}/froths_at"),
            self.froths_at.0.0,
            0.05,
            100.0,
            "meters a second",
        );
    }

    /// How long a ripple lasts under a wind of `strength`, from calm at 0 to a gale at 1.
    fn lasts_under(&self, strength: f32) -> f32 {
        self.lasts.0 + (self.lasts_in_gale.0 - self.lasts.0) * strength.clamp(0.0, 1.0)
    }
}

impl RippleSource {
    pub fn check(&self, pointer: &str, cx: &mut RowCheck) {
        cx.range(
            format!("{pointer}/every"),
            self.every.0,
            0.01,
            600.0,
            "seconds",
        );
        cx.range(
            format!("{pointer}/scatter"),
            self.scatter.0,
            0.0,
            50.0,
            "meters",
        );
        cx.range(format!("{pointer}/size"), self.size.0, 0.01, 5.0, "meters");
        cx.range(
            format!("{pointer}/strength"),
            self.strength,
            0.0,
            1.0,
            "of flesh as wide come into the water",
        );
    }
}

impl Default for RippleField {
    fn default() -> RippleField {
        RippleField {
            corner: IVec2::ZERO,
            now: vec![0.0; RIPPLE_SIDE * RIPPLE_SIDE],
            before: vec![0.0; RIPPLE_SIDE * RIPPLE_SIDE],
            next: vec![0.0; RIPPLE_SIDE * RIPPLE_SIDE],
            froth: vec![0.0; RIPPLE_SIDE * RIPPLE_SIDE],
            owed: 0.0,
            quiet: None,
            unpublished: false,
        }
    }
}

impl RippleField {
    /// The ground point of its north-west corner.
    pub fn corner(&self) -> Vec2 {
        self.corner.as_vec2() * RIPPLE_CELL.0
    }

    /// How far the water stands over its level at the ground point `at`, under it where
    /// negative; nothing off the field.
    pub fn height_at(&self, at: Vec2) -> f32 {
        let cell = (at / RIPPLE_CELL.0).floor().as_ivec2() - self.corner;
        let side = RIPPLE_SIDE as i32;
        if cell.min_element() < 0 || cell.max_element() >= side {
            return 0.0;
        }
        self.now[cell.y as usize * RIPPLE_SIDE + cell.x as usize]
    }

    /// How churned the water is at the ground point `at`, from still at 0 to all froth at 1.
    pub fn froth_at(&self, at: Vec2) -> f32 {
        let cell = (at / RIPPLE_CELL.0).floor().as_ivec2() - self.corner;
        let side = RIPPLE_SIDE as i32;
        if cell.min_element() < 0 || cell.max_element() >= side {
            return 0.0;
        }
        self.froth[cell.y as usize * RIPPLE_SIDE + cell.x as usize]
    }

    /// Keeps the field round `middle`, carrying its ripples along where it moves: it follows
    /// once the middle has strayed from its own, so it moves seldom.
    pub fn follow(&mut self, middle: Vec2) {
        let wanted =
            (middle / RIPPLE_CELL.0).floor().as_ivec2() - IVec2::splat(RIPPLE_SIDE as i32 / 2);
        let moved = wanted - self.corner;
        if moved.abs().max_element() <= STRAYS {
            return;
        }
        self.corner = wanted;
        if self.quiet.is_none() {
            return;
        }
        if moved.abs().max_element() >= RIPPLE_SIDE as i32 {
            self.still();
            return;
        }
        for held in [&mut self.now, &mut self.before, &mut self.froth] {
            carry(held, moved);
        }
        self.unpublished = true;
    }

    /// Breaks the water at the ground point `at`, over a place `size` wide and `strength`
    /// strong: the water is pushed up there, and falls back as a ring.
    pub fn strike(&mut self, at: Vec2, size: Meters, strength: f32) {
        let reach = (size.0 * 0.5).max(FINEST);
        // A place too small for the field to hold is laid as wide as it can hold one, with as
        // much less water as it is smaller.
        let pushed = strength * (size.0 * 0.5 / reach).powi(2);
        self.raise(
            at - reach,
            at + reach,
            |point| {
                let off = point.distance_squared(at) / (reach * reach);
                pushed * (1.0 - off).max(0.0).powi(2)
            },
            f32::abs,
        );
    }

    /// Moves a body's flesh through the water, from where it lay in the surface `over` ago to
    /// where it lies now: the water rises where flesh came into the surface and falls where it
    /// left, and is churned by how fast the flesh went through it, as `ripples` tell. A body
    /// newly in the water has no `was`, one that left it no `now`.
    pub fn part(
        &mut self,
        was: Option<Wading>,
        now: Option<Wading>,
        over: Seconds,
        ripples: &Ripples,
    ) {
        let cuts = |wading: Option<Wading>| -> Vec<(TreadingLimb, f32)> {
            wading.map_or_else(Vec::new, |wading| {
                wading
                    .limbs
                    .iter()
                    .filter(|limb| limb.cuts(wading.level))
                    .map(|limb| (*limb, wading.level))
                    .collect()
            })
        };
        let (was, now) = (cuts(was), cuts(now));
        let bounds = was
            .iter()
            .chain(&now)
            .map(|(limb, _)| limb.over())
            .reduce(|(least, most), (from, to)| (least.min(from), most.max(to)));
        let Some((from, to)) = bounds else {
            return;
        };
        let flesh = |cut: &[(TreadingLimb, f32)], point: Vec2| {
            cut.iter()
                .map(|(limb, level)| limb.flesh_at(point, *level))
                .fold(0.0, f32::max)
        };
        // Flesh fills a cell over the width of its edge, so how much of one it filled tells
        // how fast it went.
        let full = ripples.froths_at.0.0 * over.0 / EDGE;
        self.raise(
            from,
            to,
            |point| flesh(&now, point) - flesh(&was, point),
            |raised| raised.abs() * (raised.abs() / full).min(1.0).powi(2),
        );
    }

    /// Lets `passed` go by: the ripples travel on and fade as `ripples` tell, sooner the
    /// stronger the `wind`, from calm at 0 to a gale at 1. Water nothing broke for long enough
    /// lies still again.
    pub fn pass(&mut self, passed: Seconds, ripples: &Ripples, wind: f32) {
        let Some(quiet) = self.quiet.as_mut() else {
            return;
        };
        let lasts = ripples.lasts_under(wind);
        *quiet += passed.0;
        if *quiet > lasts.max(ripples.froth.0) * SPENT_AFTER {
            self.still();
            return;
        }
        self.owed += passed.0;
        let steps = ((self.owed / STEP) as usize).min(MOST_STEPS);
        self.owed = (self.owed - steps as f32 * STEP).min(STEP);
        let spreads = (ripples.speed.0.0.min(FASTEST_RIPPLE.0.0) * STEP / RIPPLE_CELL.0).powi(2);
        let keeps = (-SPENT_LENGTHS * STEP / lasts).exp();
        for _ in 0..steps {
            self.step(spreads, keeps);
        }
        let settles = (-SPENT_LENGTHS * STEP * steps as f32 / ripples.froth.0).exp();
        for froth in &mut self.froth {
            *froth *= settles;
        }
        self.unpublished |= steps > 0;
    }

    /// The water's rise and fall and its froth as they stand, if they changed since they were
    /// last asked for.
    pub fn unpublished(&mut self) -> Option<(&[f32], &[f32])> {
        std::mem::take(&mut self.unpublished).then_some((&self.now, &self.froth))
    }

    fn still(&mut self) {
        self.now.fill(0.0);
        self.before.fill(0.0);
        self.froth.fill(0.0);
        self.owed = 0.0;
        self.quiet = None;
        self.unpublished = true;
    }

    /// Raises the water by what `by` tells at each cell's middle between the ground points
    /// `from` and `to`, moving it without setting it in motion: what is raised falls back and
    /// spreads on its own, and churned by what `churns` makes of how far it was raised.
    fn raise(
        &mut self,
        from: Vec2,
        to: Vec2,
        by: impl Fn(Vec2) -> f32,
        churns: impl Fn(f32) -> f32,
    ) {
        let cell = RIPPLE_CELL.0;
        let first = ((from / cell).floor().as_ivec2() - self.corner).max(IVec2::ONE);
        let last = ((to / cell).floor().as_ivec2() - self.corner)
            .min(IVec2::splat(RIPPLE_SIDE as i32 - 2));
        let mut broken = false;
        for row in first.y..=last.y {
            for column in first.x..=last.x {
                let middle = (IVec2::new(column, row) + self.corner).as_vec2() * cell + cell * 0.5;
                let raised = by(middle);
                if raised == 0.0 {
                    continue;
                }
                let index = row as usize * RIPPLE_SIDE + column as usize;
                self.now[index] += raised;
                self.before[index] += raised;
                self.froth[index] = (self.froth[index] + churns(raised)).min(1.0);
                broken = true;
            }
        }
        if broken {
            self.quiet = Some(0.0);
            self.unpublished = true;
        }
    }

    /// Carries the ripples on by one [`STEP`]: each cell is drawn toward the cells round it,
    /// `spreads` of the way, its motion evened out with theirs by [`EVENS`], which wears the
    /// finest ripples away first as water does, and all of it is left `keeps` of what it was.
    /// The field's rim stays level, so a ripple that reaches it is turned back, long since
    /// faded.
    fn step(&mut self, spreads: f32, keeps: f32) {
        let side = RIPPLE_SIDE;
        for row in 1..side - 1 {
            let [north, here, south] = rows_round(&self.now, row);
            let [north_before, before, south_before] = rows_round(&self.before, row);
            let now = north.zip(here).zip(south);
            let was = north_before.zip(before).zip(south_before);
            let cells = self.next[row * side + 1..(row + 1) * side - 1].iter_mut();
            for ((next, ((north, here), south)), ((north_before, before), south_before)) in
                cells.zip(now).zip(was)
            {
                *next = keeps * (2.0 * here[1] + (spreads + EVENS) * around(north, here, south))
                    - keeps
                        * keeps
                        * (before[1] + EVENS * around(north_before, before, south_before));
            }
        }
        std::mem::swap(&mut self.before, &mut self.now);
        std::mem::swap(&mut self.now, &mut self.next);
    }
}

impl TreadingLimb {
    /// Whether a surface at `level` cuts through the limb's flesh, or lies near enough to.
    fn cuts(&self, level: f32) -> bool {
        let girth = self.girth.max(self.tail_girth) + EDGE * 0.5;
        self.from.y.min(self.to.y) - girth < level && level < self.from.y.max(self.to.y) + girth
    }

    /// The ground the limb's flesh lies over at most, from its north-west to its south-east.
    fn over(&self) -> (Vec2, Vec2) {
        let girth = Vec2::splat(self.girth.max(self.tail_girth) + EDGE * 0.5);
        let (from, to) = (self.from.xz(), self.to.xz());
        (from.min(to) - girth, from.max(to) + girth)
    }

    /// How much of the surface at `level` is flesh of the limb at the ground point `point`,
    /// from none to all of it, across the [`EDGE`] round the flesh.
    fn flesh_at(&self, point: Vec2, level: f32) -> f32 {
        let at = Vec3::new(point.x, level, point.y);
        let bone = self.to - self.from;
        let long = bone.length_squared();
        let along = if long > 1e-8 {
            ((at - self.from).dot(bone) / long).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let girth = self.girth + (self.tail_girth - self.girth) * along;
        let away = at.distance(self.from + bone * along) - girth;
        (0.5 - away / EDGE).clamp(0.0, 1.0)
    }
}

/// What the ripples are stirred by: the moment, the game's ripples, who treads the water, what
/// keeps it broken, the rain and the wind, and where the camera looks.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct Stirring<'w, 's> {
    time: Res<'w, Time>,
    content: Option<Res<'w, Content>>,
    treading: Res<'w, Treading>,
    spots: Query<'w, 's, (&'static GlobalTransform, &'static RippleSpot)>,
    climate: Option<Res<'w, ShownWeather>>,
    rain: Res<'w, RainField>,
    shelter: Res<'w, Shelter>,
    rig: Option<Res<'w, CameraRig>>,
    ground: Option<Res<'w, GroundHeights>>,
}

/// Every frame, breaks the water where the bodies treading it moved through its surface, where
/// the rain strikes it and where a thing keeps it broken, lets the ripples travel on, and
/// publishes the field where it changed.
pub(crate) fn stir(
    sources: Stirring,
    mut field: ResMut<RippleField>,
    mut write: ResMut<ShaderFieldWrite>,
    mut waded: Local<HashMap<Entity, Waded>>,
    mut rng: Local<Option<Rng>>,
) {
    let Stirring {
        time,
        content,
        treading,
        spots,
        climate,
        rain,
        shelter,
        rig,
        ground,
    } = sources;
    let ripples = content.and_then(|content| content.built::<GameSettings>().ripples);
    let (Some(ripples), Some(ground), Some(rig)) = (ripples, ground, rig) else {
        return;
    };
    let field = &mut *field;
    let rng = rng.get_or_insert_with(Rng::from_entropy);
    let passed = time.delta_secs();
    field.follow(rig.target().xz());
    waded.retain(|entity, _| treading.0.iter().any(|body| body.entity == *entity));
    for body in &treading.0 {
        let now = Waded {
            limbs: body.limbs.clone(),
            level: body.water,
            put: body.put,
        };
        // A body put where it is came no way through the water to stir it along.
        if let Some(was) = waded.get(&body.entity)
            && was.put == now.put
            && (was.limbs != now.limbs || was.level != now.level)
        {
            field.part(was.wading(), now.wading(), Seconds(passed), &ripples);
        }
        waded.insert(body.entity, now);
    }
    let level_at = |point: Vec2| {
        let at = ground_pos(Vec3::new(point.x, 0.0, point.y));
        (ground.water.depth_at(&ground.heights, at).0 > 0.0).then(|| ground.water.height_at(at).0)
    };
    for (placed, spot) in &spots {
        if rng.rand_float() * spot.0.every.0 >= passed {
            continue;
        }
        let turn = rng.rand_float() * std::f32::consts::TAU;
        let off = Vec2::from_angle(turn) * rng.rand_float().sqrt() * spot.0.scatter.0;
        let at = placed.translation().xz() + off;
        if level_at(at).is_some() {
            field.strike(at, spot.0.size, spot.0.strength);
        }
    }
    let wind = climate
        .as_ref()
        .map_or(0.0, |climate| climate.wind.length());
    if let Some(drops) = climate.and_then(|climate| climate.ripples) {
        let side = RIPPLE_SIDE as f32 * RIPPLE_CELL.0;
        let patches = (side / drops.scatter.0.max(RIPPLE_CELL.0)).powi(2);
        let struck = (patches * passed / drops.every.0 + rng.rand_float()).min(MOST_DROPS);
        for _ in 0..struck as usize {
            let at = field.corner() + Vec2::new(rng.rand_float(), rng.rand_float()) * side;
            if rng.rand_float() >= rain.rain_at(at) {
                continue;
            }
            let Some(level) = level_at(at) else {
                continue;
            };
            if !shelter.covers(Vec3::new(at.x, level, at.y)) {
                field.strike(at, drops.size, drops.strength);
            }
        }
    }
    field.pass(Seconds(passed), &ripples, wind);
    if let Some((heights, froth)) = field.unpublished() {
        *write = ShaderFieldWrite::holding(heights, froth);
    }
}

/// What a body's flesh was like the frame before: its limbs, the level of the water it stood or
/// floated in, none on dry ground, and the count of the times it was put where it stood.
pub(crate) struct Waded {
    limbs: Vec<TreadingLimb>,
    level: Option<f32>,
    put: Option<Jumps>,
}

impl Waded {
    fn wading(&self) -> Option<Wading<'_>> {
        self.level.map(|level| Wading {
            limbs: &self.limbs,
            level,
        })
    }
}

/// The row `row` of `held` with the rows to its north and south, each as every three cells that
/// lie side by side in it.
fn rows_round(held: &[f32], row: usize) -> [std::slice::Windows<'_, f32>; 3] {
    [row - 1, row, row + 1].map(|row| held[row * RIPPLE_SIDE..(row + 1) * RIPPLE_SIDE].windows(3))
}

/// How far the mean of the cells round the middle one of `here` stands over it, the rows to its
/// `north` and `south` given: the cells straight across weigh four times the ones cornerwise,
/// which spreads a ripple alike every way over the grid.
fn around(north: &[f32], here: &[f32], south: &[f32]) -> f32 {
    (4.0 * (here[0] + here[2] + north[1] + south[1]) + north[0] + north[2] + south[0] + south[2]
        - 20.0 * here[1])
        / 6.0
}

/// Carries what `held` holds to where it lies once the field's corner has `moved` that many
/// cells, levelling the cells that came into the field and its rim.
fn carry(held: &mut [f32], moved: IVec2) {
    let side = RIPPLE_SIDE;
    let offset = moved.y as isize * side as isize + moved.x as isize;
    let size = held.len();
    if offset > 0 {
        held.copy_within(offset as usize.., 0);
    } else {
        held.copy_within(..size - offset.unsigned_abs(), offset.unsigned_abs());
    }
    let entered = |moved: i32| -> Range<usize> {
        let cells = moved.unsigned_abs() as usize;
        if moved > 0 {
            side - cells..side
        } else {
            0..cells
        }
    };
    let (rows, columns) = (entered(moved.y), entered(moved.x));
    held[rows.start * side..rows.end * side].fill(0.0);
    for row in held.chunks_exact_mut(side) {
        row[columns.clone()].fill(0.0);
        row[0] = 0.0;
        row[side - 1] = 0.0;
    }
    held[..side].fill(0.0);
    held[size - side..].fill(0.0);
}

/// How long a step of the field is, in seconds, and how many of them one frame runs at most: a
/// frame that took longer leaves the ripples that much behind.
const STEP: f32 = 1.0 / 60.0;
const MOST_STEPS: usize = 4;

/// How many lengths of fading leave a twentieth of a ripple, and after how many times as long
/// as a ripple lasts unbroken water lies still again.
const SPENT_LENGTHS: f32 = 3.0;
const SPENT_AFTER: f32 = 2.0;

/// How far from the field's middle, in cells, the camera's subject strays before the field
/// follows it.
const STRAYS: i32 = RIPPLE_SIDE as i32 / 8;

/// How wide the edge of flesh is laid in the surface, and the narrowest place a strike is laid
/// over, from its middle: what the field's cells can hold without a ripple taking their shape.
const EDGE: f32 = RIPPLE_CELL.0 * 1.5;
const FINEST: f32 = RIPPLE_CELL.0 * 2.0;

/// How much of the way a cell's motion is evened out with that of the cells round it in a step.
const EVENS: f32 = 0.06;

/// How many drops of rain strike the field in a frame at most.
const MOST_DROPS: f32 = 256.0;
