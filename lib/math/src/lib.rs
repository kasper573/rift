//! Generic math and the repo's spatial-unit vocabulary. [`Pos`]/[`Rect`] are generic over a [`Unit`]
//! ([`Pixels`], [`Tiles`]) so a pixel value can never be used where a tile value is meant; the same
//! types are reused by the map loader, sprite models, the image atlas, and the game.

use std::f32::consts::FRAC_1_SQRT_2;

use rift::Wire;
use serde::Deserialize;

/// A scalar in a fixed unit, convertible to and from its raw `f32` so generic [`Pos`]/[`Rect`] can do
/// arithmetic without knowing the unit.
pub trait Unit: Copy {
    fn raw(self) -> f32;
    fn of(value: f32) -> Self;
}

/// A length or coordinate in a map's pixel space — Tiled authors object geometry (and its own tile
/// size) in pixels. Cross into tile space only through [`Pixels::to_tiles`] / [`Tiling`].
#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd, Deserialize)]
pub struct Pixels(pub f32);

/// A length or coordinate in tile space — the game's spatial unit (positions, ranges, distances, map
/// extents). Whole numbers fall on tile edges; tile centers are at +0.5.
#[derive(Wire, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct Tiles(pub f32);

impl Pixels {
    /// Pixels → tiles, given the map's pixels-per-tile.
    pub fn to_tiles(self, per_tile: Pixels) -> Tiles {
        Tiles(self.0 / per_tile.0)
    }
}

impl Unit for Pixels {
    fn raw(self) -> f32 {
        self.0
    }
    fn of(value: f32) -> Pixels {
        Pixels(value)
    }
}

impl Unit for Tiles {
    fn raw(self) -> f32 {
        self.0
    }
    fn of(value: f32) -> Tiles {
        Tiles(value)
    }
}

/// A dimensionless count (e.g. inventory slots) is a unit too, so a [`Size<u32>`] grid can scale into
/// pixels via [`Vec2::convert`] / [`Vec2::mult`].
impl Unit for u32 {
    fn raw(self) -> f32 {
        self as f32
    }
    fn of(value: f32) -> u32 {
        value as u32
    }
}

impl std::ops::Sub for Pixels {
    type Output = Pixels;
    fn sub(self, other: Pixels) -> Pixels {
        Pixels(self.0 - other.0)
    }
}

/// A movement speed: the tiles covered each second.
#[derive(Wire, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct TilesPerSec(pub Tiles);

impl TilesPerSec {
    /// The distance covered in `dt` seconds.
    pub fn over(self, dt: f32) -> Tiles {
        Tiles(self.0.0 * dt)
    }
}

/// A duration or server-clock timestamp in seconds.
#[derive(Wire, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct Seconds(pub f32);

/// A duration in milliseconds.
#[derive(Wire, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct Millis(pub f32);

/// An animation-rate multiplier; 1 plays the animation at its authored speed.
#[derive(Wire, Deserialize, Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct PlaybackRate(pub f32);

impl std::ops::Add for Seconds {
    type Output = Seconds;
    fn add(self, other: Seconds) -> Seconds {
        Seconds(self.0 + other.0)
    }
}

impl std::ops::Sub for Seconds {
    type Output = Seconds;
    fn sub(self, other: Seconds) -> Seconds {
        Seconds(self.0 - other.0)
    }
}

impl std::ops::Mul<f32> for TilesPerSec {
    type Output = TilesPerSec;
    fn mul(self, factor: f32) -> TilesPerSec {
        TilesPerSec(Tiles(self.0.0 * factor))
    }
}

impl Millis {
    pub fn seconds(self) -> Seconds {
        Seconds(self.0 / 1000.0)
    }
}

/// The 2-component vector backing [`Pos`] and [`Size`] — an implementation detail. Code names it
/// `Pos` (a point or displacement) or `Size` (a width in `.x`, height in `.y`), never `Vec2`.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Deserialize)]
pub struct Vec2<U> {
    pub x: U,
    pub y: U,
}

/// A 2D position or displacement over a unit `U` ([`Pos<Tiles>`], [`Pos<Pixels>`]).
pub type Pos<U> = Vec2<U>;
/// A 2D size — width in `.x`, height in `.y` — over a unit `U`; replaces ad-hoc `(w, h)` tuples.
pub type Size<U> = Vec2<U>;

impl<U> Vec2<U> {
    pub const fn new(x: U, y: U) -> Vec2<U> {
        Vec2 { x, y }
    }
}

impl<U: Copy> Vec2<U> {
    /// Both components set to `value` — a square [`Size`] or a uniform offset.
    pub const fn splat(value: U) -> Vec2<U> {
        Vec2 { x: value, y: value }
    }
}

impl<U: Unit> Vec2<U> {
    /// Every component multiplied by `factor`, keeping the unit.
    pub fn scale(self, factor: f32) -> Vec2<U> {
        self.map(|v| v * factor)
    }

    /// Every component mapped through `f`, keeping the unit.
    pub fn map(self, f: impl Fn(f32) -> f32) -> Vec2<U> {
        Vec2::new(U::of(f(self.x.raw())), U::of(f(self.y.raw())))
    }

    /// Every component mapped through `f` and reinterpreted in unit `V` — the one gateway between
    /// units (e.g. tiles → pixels by the tile size), so a bare reunit can't slip in silently.
    pub fn convert<V: Unit>(self, f: impl Fn(f32) -> f32) -> Vec2<V> {
        Vec2::new(V::of(f(self.x.raw())), V::of(f(self.y.raw())))
    }

    /// Componentwise product, keeping this vector's unit (e.g. a per-tile pixel size times a tile
    /// count gives a pixel size).
    pub fn mult<V: Unit>(self, other: Vec2<V>) -> Vec2<U> {
        Vec2::new(
            U::of(self.x.raw() * other.x.raw()),
            U::of(self.y.raw() * other.y.raw()),
        )
    }

    /// Componentwise minimum.
    pub fn min(self, other: Vec2<U>) -> Vec2<U> {
        Vec2::new(
            U::of(self.x.raw().min(other.x.raw())),
            U::of(self.y.raw().min(other.y.raw())),
        )
    }

    /// Componentwise maximum.
    pub fn max(self, other: Vec2<U>) -> Vec2<U> {
        Vec2::new(
            U::of(self.x.raw().max(other.x.raw())),
            U::of(self.y.raw().max(other.y.raw())),
        )
    }

    /// Each component clamped to `[lo, hi]` on its own axis.
    pub fn clamp(self, lo: Vec2<U>, hi: Vec2<U>) -> Vec2<U> {
        Vec2::new(
            U::of(self.x.raw().clamp(lo.x.raw(), hi.x.raw())),
            U::of(self.y.raw().clamp(lo.y.raw(), hi.y.raw())),
        )
    }

    pub fn length(self) -> f32 {
        self.x.raw().hypot(self.y.raw())
    }

    pub fn distance(self, other: Vec2<U>) -> f32 {
        (self - other).length()
    }

    pub fn normalized(self) -> Vec2<U> {
        let length = self.length();
        if length > 0.0 {
            self.scale(1.0 / length)
        } else {
            Vec2::new(U::of(0.0), U::of(0.0))
        }
    }
}

impl<U: Unit> std::ops::Add for Vec2<U> {
    type Output = Vec2<U>;
    fn add(self, other: Vec2<U>) -> Vec2<U> {
        Vec2::new(
            U::of(self.x.raw() + other.x.raw()),
            U::of(self.y.raw() + other.y.raw()),
        )
    }
}

impl<U: Unit> std::ops::Sub for Vec2<U> {
    type Output = Vec2<U>;
    fn sub(self, other: Vec2<U>) -> Vec2<U> {
        Vec2::new(
            U::of(self.x.raw() - other.x.raw()),
            U::of(self.y.raw() - other.y.raw()),
        )
    }
}

// Hand-written because `#[derive(Wire)]` cannot carry the `<U>` generic. Lets a component or event
// hold a `Pos`/`Size` directly instead of re-spelling its `x`/`y` fields.
impl<U: Wire> Wire for Vec2<U> {
    fn encode(&self, out: &mut Vec<u8>) {
        self.x.encode(out);
        self.y.encode(out);
    }

    fn decode(bytes: &mut &[u8]) -> Option<Vec2<U>> {
        Some(Vec2 {
            x: U::decode(bytes)?,
            y: U::decode(bytes)?,
        })
    }
}

/// An axis-aligned rectangle: a [`Pos`] corner and a [`Size`] extent. `Eq` applies only to integer
/// units (e.g. an atlas `Rect<u32>`); float units get `PartialEq` alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rect<U> {
    pub pos: Pos<U>,
    pub size: Size<U>,
}

impl<U> Rect<U> {
    pub fn new(pos: Pos<U>, size: Size<U>) -> Rect<U> {
        Rect { pos, size }
    }
}

impl<U: Unit> Rect<U> {
    pub fn contains(self, point: Pos<U>) -> bool {
        point.x.raw() >= self.pos.x.raw()
            && point.y.raw() >= self.pos.y.raw()
            && point.x.raw() < self.pos.x.raw() + self.size.x.raw()
            && point.y.raw() < self.pos.y.raw() + self.size.y.raw()
    }

    pub fn center(self) -> Pos<U> {
        self.pos + self.size.scale(0.5)
    }

    /// The same rectangle shifted by a displacement.
    pub fn translate(self, by: Vec2<U>) -> Rect<U> {
        Rect::new(self.pos + by, self.size)
    }

    /// The overlapping rectangle with `other`, or `None` if they are disjoint.
    pub fn intersection(self, other: Rect<U>) -> Option<Rect<U>> {
        let min = Pos::new(
            U::of(self.pos.x.raw().max(other.pos.x.raw())),
            U::of(self.pos.y.raw().max(other.pos.y.raw())),
        );
        let far = |r: Rect<U>| {
            (
                r.pos.x.raw() + r.size.x.raw(),
                r.pos.y.raw() + r.size.y.raw(),
            )
        };
        let (sx, sy) = far(self);
        let (ox, oy) = far(other);
        let size = Size::new(
            U::of(sx.min(ox) - min.x.raw()),
            U::of(sy.min(oy) - min.y.raw()),
        );
        (size.x.raw() > 0.0 && size.y.raw() > 0.0).then_some(Rect::new(min, size))
    }

    /// Width times height.
    pub fn area(self) -> f32 {
        self.size.x.raw() * self.size.y.raw()
    }
}

/// A map's pixels-per-tile, and the only gateway from its pixel space into tile space. Built once per
/// map so the tile size has a single source.
#[derive(Clone, Copy)]
pub struct Tiling {
    tile_width: Pixels,
    tile_height: Pixels,
}

impl Tiling {
    pub fn new(tile_width: Pixels, tile_height: Pixels) -> Tiling {
        Tiling {
            tile_width: Pixels(tile_width.0.max(1.0)),
            tile_height: Pixels(tile_height.0.max(1.0)),
        }
    }

    /// A pixel vector (a point or a size) in tile space; whole numbers lie on tile edges.
    pub fn point(self, p: Pos<Pixels>) -> Pos<Tiles> {
        Pos::new(
            p.x.to_tiles(self.tile_width),
            p.y.to_tiles(self.tile_height),
        )
    }

    /// A pixel rect in tile space.
    pub fn rect(self, r: Rect<Pixels>) -> Rect<Tiles> {
        Rect::new(self.point(r.pos), self.point(r.size))
    }

    /// The center of the tile a pixel point falls in. Snapping a loosely-authored pixel spawn point
    /// to a whole tile is what keeps a placed actor on the tile grid that movement rests it on.
    pub fn tile_center(self, p: Pos<Pixels>) -> Pos<Tiles> {
        let t = self.point(p);
        Pos::new(Tiles(t.x.0.floor() + 0.5), Tiles(t.y.0.floor() + 0.5))
    }
}

/// One of 8 compass facings; the discriminant is the sprite-strip index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    S = 0,
    SW = 1,
    NW = 2,
    N = 3,
    NE = 4,
    SE = 5,
    E = 6,
    W = 7,
}

impl Direction {
    /// The facing nearest a displacement; a zero vector faces south. Takes raw components — a facing
    /// is dimensionless, so it is independent of the displacement's unit.
    pub fn from_vec(dx: f32, dy: f32) -> Direction {
        if dx == 0.0 && dy == 0.0 {
            return Direction::S;
        }
        const D: f32 = FRAC_1_SQRT_2;
        let candidates = [
            (Direction::E, 1.0, 0.0),
            (Direction::SE, D, D),
            (Direction::S, 0.0, 1.0),
            (Direction::SW, -D, D),
            (Direction::W, -1.0, 0.0),
            (Direction::NW, -D, -D),
            (Direction::N, 0.0, -1.0),
            (Direction::NE, D, -D),
        ];
        let mut best = Direction::S;
        let mut best_dot = f32::NEG_INFINITY;
        for (dir, cx, cy) in candidates {
            let dot = dx * cx + dy * cy;
            if dot > best_dot {
                best_dot = dot;
                best = dir;
            }
        }
        best
    }

    pub fn from_u8(value: u8) -> Direction {
        match value {
            1 => Direction::SW,
            2 => Direction::NW,
            3 => Direction::N,
            4 => Direction::NE,
            5 => Direction::SE,
            6 => Direction::E,
            7 => Direction::W,
            _ => Direction::S,
        }
    }

    pub fn norm_angle(self) -> f32 {
        use std::f32::consts::PI;
        let angle = match self {
            Direction::E => 0.0,
            Direction::SE => PI / 4.0,
            Direction::S => PI / 2.0,
            Direction::SW => 3.0 * PI / 4.0,
            Direction::W => PI,
            Direction::NW => -3.0 * PI / 4.0,
            Direction::N => -PI / 2.0,
            Direction::NE => -PI / 4.0,
        };
        if angle < 0.0 { angle + 2.0 * PI } else { angle }
    }
}

/// A deterministic xorshift rng state, shared by spawning and reward rolls.
pub struct Rng(pub u64);

pub fn next_rng(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

pub fn rng_unit(state: &mut u64) -> f32 {
    (next_rng(state) >> 40) as f32 / (1u64 << 24) as f32
}
