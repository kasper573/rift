use std::hash::{Hash, Hasher};
use std::ops::{Div, Mul, Rem};
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::SchemaMark;
use crate::core::math::{Offset, Pos, Rect, Rng, Size};

#[derive(
    JsonSchema,
    Serialize,
    Deserialize,
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    PartialOrd,
    derive_more::Add,
    derive_more::Sub,
    derive_more::SubAssign,
)]
#[schemars(transform = SchemaMark::Unit.with("m"))]
pub struct Meters(pub f32);

/// An amount of memory.
#[derive(
    Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord,
)]
pub struct Bytes(pub u64);

impl Bytes {
    pub fn megabytes(self) -> f64 {
        self.0 as f64 / (1024.0 * 1024.0)
    }
}

/// Stops of light: each one doubles it.
#[derive(
    JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd,
)]
#[schemars(transform = SchemaMark::Unit.with("EV"))]
pub struct Stops(pub f32);

impl Stops {
    pub fn is_zero(&self) -> bool {
        self.0 == 0.0
    }
}

pub type CellPos = euclid::default::Point2D<i32>;
pub type GridSize = euclid::default::Size2D<u32>;

/// The corner of an area's ground: cells are centred on whole metres, so the cell `(0, 0)` and the
/// ground begin half a metre before the origin.
pub const GROUND_ORIGIN: Pos<Meters> = Pos::new(-0.5, -0.5);

#[derive(
    JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd,
)]
pub struct MetersPerSec(pub Meters);

impl Meters {
    pub fn ratio(self, other: Meters) -> f32 {
        self.0 / other.0
    }

    pub fn is_zero(&self) -> bool {
        self.0 == 0.0
    }
}

impl Mul<f32> for MetersPerSec {
    type Output = MetersPerSec;
    fn mul(self, factor: f32) -> MetersPerSec {
        MetersPerSec(Meters(self.0.0 * factor))
    }
}

impl Mul<Seconds> for MetersPerSec {
    type Output = Meters;
    fn mul(self, time: Seconds) -> Meters {
        Meters(self.0.0 * time.0)
    }
}

/// A metre cell of an area's ground, the grain its terrain is painted and sloped at.
pub trait GroundCell {
    fn center(self) -> Pos<Meters>;
    fn bounds(self) -> Rect<Meters>;
    fn index(self, size: GridSize) -> Option<usize>;
}

impl GroundCell for CellPos {
    fn center(self) -> Pos<Meters> {
        Pos::new(self.x as f32, self.y as f32)
    }

    fn bounds(self) -> Rect<Meters> {
        Rect::new(
            Pos::new(self.x as f32 - 0.5, self.y as f32 - 0.5),
            Size::splat(1.0),
        )
    }

    fn index(self, size: GridSize) -> Option<usize> {
        let (w, h) = (size.width as i32, size.height as i32);
        let inside = self.x >= 0 && self.y >= 0 && self.x < w && self.y < h;
        inside.then(|| (self.y * w + self.x) as usize)
    }
}

pub trait MeterPos {
    fn distance(self, other: Pos<Meters>) -> Meters;
    fn toward(self, target: Pos<Meters>, by: Meters) -> Pos<Meters>;
    fn hitbox(self, size: Size<Meters>) -> Rect<Meters>;
}

impl MeterPos for Pos<Meters> {
    fn distance(self, other: Pos<Meters>) -> Meters {
        Meters(self.distance_to(other))
    }

    fn toward(self, target: Pos<Meters>, by: Meters) -> Pos<Meters> {
        self + (target - self).normalize() * by.0
    }

    fn hitbox(self, size: Size<Meters>) -> Rect<Meters> {
        Rect::new(
            self + Offset::new(-size.width / 2.0, 0.5 - size.height),
            size,
        )
    }
}

pub trait MeterSize {
    fn bounds(self) -> Rect<Meters>;
    fn grid(self) -> GridSize;
}

impl MeterSize for Size<Meters> {
    fn bounds(self) -> Rect<Meters> {
        Rect::new(GROUND_ORIGIN, self)
    }

    fn grid(self) -> GridSize {
        GridSize::new(self.width as u32, self.height as u32)
    }
}

pub trait GridDims {
    fn cells(self) -> impl Iterator<Item = CellPos>;
    /// The cell under `at`, none off the grid.
    fn cell_under(self, at: Pos<Meters>) -> Option<CellPos>;
}

impl GridDims for GridSize {
    fn cells(self) -> impl Iterator<Item = CellPos> {
        let (w, h) = (self.width as i32, self.height as i32);
        (0..h).flat_map(move |y| (0..w).map(move |x| CellPos::new(x, y)))
    }

    fn cell_under(self, at: Pos<Meters>) -> Option<CellPos> {
        let cell = CellPos::new(at.x.round() as i32, at.y.round() as i32);
        cell.index(self).map(|_| cell)
    }
}

#[derive(
    JsonSchema,
    Serialize,
    Deserialize,
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    PartialOrd,
    derive_more::Add,
    derive_more::AddAssign,
    derive_more::Sub,
    derive_more::SubAssign,
)]
#[schemars(transform = SchemaMark::Unit.with("s"))]
pub struct Seconds(pub f32);

#[derive(
    JsonSchema,
    Serialize,
    Deserialize,
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    PartialOrd,
    derive_more::Add,
    derive_more::AddAssign,
    derive_more::Sub,
    derive_more::SubAssign,
)]
#[schemars(transform = SchemaMark::Unit.with("ms"))]
pub struct Millis(pub f32);

/// An hour on the clock of a game's day, 0 to 24.
#[derive(
    JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd,
)]
#[schemars(transform = SchemaMark::Unit.with("o'clock"))]
pub struct HourOfDay(pub f32);

impl HourOfDay {
    pub const NOON: HourOfDay = HourOfDay(12.0);

    /// The hour the clock shows `hours` after midnight, however many days that is.
    pub fn wrapped(hours: f32) -> HourOfDay {
        HourOfDay(hours.rem_euclid(24.0))
    }

    /// The hour as a share of the day.
    pub fn share(self) -> f32 {
        self.0 / 24.0
    }

    /// How long it has been `self` o'clock since it last was `earlier` o'clock.
    pub fn since(self, earlier: HourOfDay) -> DayHours {
        DayHours((self.0 - earlier.0).rem_euclid(24.0))
    }
}

/// A stretch of a game's day, in its hours.
#[derive(
    JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd,
)]
#[schemars(transform = SchemaMark::Unit.with("h"))]
pub struct DayHours(pub f32);

impl DayHours {
    pub fn is_zero(&self) -> bool {
        self.0 == 0.0
    }
}

#[derive(
    JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd,
)]
pub struct PlaybackRate(pub f32);

#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, PartialOrd)]
#[schemars(transform = SchemaMark::Unit.with("Hz"))]
pub struct Hertz(pub f32);

impl Hertz {
    pub fn period(self) -> Duration {
        Duration::from_secs_f32(1.0 / self.0)
    }
}

impl PlaybackRate {
    pub fn at_least(self, floor: f32) -> PlaybackRate {
        PlaybackRate(self.0.max(floor))
    }
}

impl Div<PlaybackRate> for Seconds {
    type Output = Seconds;
    fn div(self, rate: PlaybackRate) -> Seconds {
        Seconds(self.0 / rate.0)
    }
}

impl Mul<PlaybackRate> for Millis {
    type Output = Millis;
    fn mul(self, rate: PlaybackRate) -> Millis {
        Millis(self.0 * rate.0)
    }
}

impl Rem for Millis {
    type Output = Millis;
    fn rem(self, other: Millis) -> Millis {
        Millis(self.0 % other.0)
    }
}

impl Hash for Seconds {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // -0.0 equals 0.0, so both must hash alike.
        (self.0 + 0.0).to_bits().hash(state);
    }
}

impl Seconds {
    /// Rounded up to whole minutes, for a countdown shown by the minute.
    pub fn up_to_minute(self) -> Seconds {
        Seconds((self.0 / 60.0).ceil() * 60.0)
    }

    pub fn is_zero(&self) -> bool {
        self.0 == 0.0
    }

    pub fn millis(self) -> Millis {
        Millis(self.0 * 1000.0)
    }

    pub fn min(self, other: Seconds) -> Seconds {
        Seconds(self.0.min(other.0))
    }

    pub fn ratio(self, other: Seconds) -> f32 {
        self.0 / other.0
    }

    pub fn in_words(self) -> String {
        let exact = self.0.ceil().max(1.0) as u32;
        let (_, small) = unit_pair(exact);
        let rounded = exact.div_ceil(small.size) * small.size;
        let (big, small) = unit_pair(rounded);
        [
            (rounded / big.size, big.name),
            (rounded % big.size / small.size, small.name),
        ]
        .into_iter()
        .filter(|&(count, _)| count > 0)
        .map(|(count, name)| match count {
            1 => format!("1 {name}"),
            _ => format!("{count} {name}s"),
        })
        .collect::<Vec<_>>()
        .join(" and ")
    }
}

impl From<Seconds> for Duration {
    fn from(seconds: Seconds) -> Duration {
        Duration::from_secs_f32(seconds.0.max(0.0))
    }
}

impl Mul<f32> for Seconds {
    type Output = Seconds;
    fn mul(self, factor: f32) -> Seconds {
        Seconds(self.0 * factor)
    }
}

impl Millis {
    pub fn seconds(self) -> Seconds {
        Seconds(self.0 / 1000.0)
    }

    pub fn min(self, other: Millis) -> Millis {
        Millis(self.0.min(other.0))
    }

    pub fn max(self, other: Millis) -> Millis {
        Millis(self.0.max(other.0))
    }

    pub fn ratio(self, other: Millis) -> f32 {
        self.0 / other.0
    }
}

#[derive(Clone, Copy)]
struct TimeUnit {
    size: u32,
    name: &'static str,
}

const TIME_UNITS: [TimeUnit; 4] = [
    TimeUnit {
        size: 86_400,
        name: "day",
    },
    TimeUnit {
        size: 3_600,
        name: "hour",
    },
    TimeUnit {
        size: 60,
        name: "minute",
    },
    TimeUnit {
        size: 1,
        name: "second",
    },
];

fn unit_pair(seconds: u32) -> (TimeUnit, TimeUnit) {
    let big = TIME_UNITS
        .iter()
        .position(|unit| seconds >= unit.size)
        .unwrap_or(TIME_UNITS.len() - 1)
        .min(TIME_UNITS.len() - 2);
    (TIME_UNITS[big], TIME_UNITS[big + 1])
}

#[derive(
    JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd,
)]
#[schemars(transform = SchemaMark::Unit.with("°"))]
pub struct Degrees(pub f32);

#[derive(
    JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd,
)]
#[schemars(transform = SchemaMark::Unit.with("rad"))]
pub struct Radians(pub f32);

#[derive(
    JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd,
)]
#[schemars(transform = SchemaMark::Unit.with("rad/s"))]
pub struct RadiansPerSec(pub f32);

impl Degrees {
    pub fn radians(self) -> Radians {
        Radians(self.0.to_radians())
    }

    pub fn is_zero(&self) -> bool {
        self.0 == 0.0
    }

    /// The angle as whole quarter turns, when it is one: a placement turns only by right angles,
    /// so what it places keeps its rectangles square to the world.
    pub fn quarter_turns(self) -> Option<u8> {
        let quarters = self.0 / 90.0;
        let whole = quarters.round();
        ((quarters - whole).abs() < 1e-4).then(|| whole.rem_euclid(4.0) as u8)
    }

    /// `offset` turned by the angle about the origin; a right angle turns it exactly.
    pub fn rotate(self, offset: Offset<Meters>) -> Offset<Meters> {
        let Offset { x, y, .. } = offset;
        match self.quarter_turns() {
            Some(0) => offset,
            Some(1) => Offset::new(-y, x),
            Some(2) => Offset::new(-x, -y),
            Some(3) => Offset::new(y, -x),
            _ => {
                let (sin, cos) = self.radians().0.sin_cos();
                Offset::new(x * cos - y * sin, x * sin + y * cos)
            }
        }
    }
}

impl Radians {
    /// Toward +y, which the camera looks down on: the way everyone first stands.
    pub const SOUTH: Radians = Radians(std::f32::consts::FRAC_PI_2);

    pub fn of(heading: Offset<Meters>) -> Radians {
        Radians(heading.y.atan2(heading.x))
    }

    pub fn heading(self) -> Offset<Meters> {
        Offset::new(self.0.cos(), self.0.sin())
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct WorldPx(pub f32);

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct WindowPx(pub f32);

#[derive(
    JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, PartialOrd, Default,
)]
#[schemars(transform = SchemaMark::Unit.with("%"))]
pub struct Percent(pub f32);

impl Percent {
    /// All of it: every time, of a chance.
    pub const WHOLE: Percent = Percent(100.0);

    pub fn whole() -> Percent {
        Percent::WHOLE
    }

    pub fn is_whole(&self) -> bool {
        *self == Percent::WHOLE
    }

    pub fn plus(self, other: Percent) -> Percent {
        Percent(self.0 + other.0)
    }

    pub fn times(self, count: u32) -> Percent {
        Percent(self.0 * count as f32)
    }

    pub fn rolled(self, rng: &mut Rng) -> bool {
        rng.rand_float() * 100.0 < self.0
    }
}
