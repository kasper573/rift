//! The grids an area's ground is made of: its size in whole metres, the height of the ground at
//! every half metre, and which terrain paints it there. Both grids are cut into tiles, each a file
//! of the area's bundle and, once loaded, a block of memory everyone holding the ground shares, so
//! an area of any size is read, written, copied and reshaped tile by tile and holds only the tiles
//! someone has come to. A coarse overview of the whole ground, a file of its own, answers for the
//! tiles not loaded. The renderer samples them, the loader checks them, and the server reads slopes
//! and blocking paint from them.

use std::io::Cursor;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use bevy::log::warn;
use rayon::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::chunk::CHUNK_SIDE;
use crate::core::content::{ContentProblem, RowCheck, StoredPart};
use crate::core::math::{Pos, Rect, Size};
use crate::core::units::{CellPos, Degrees, GROUND_ORIGIN, GridDims, GridSize, MeterSize, Meters};

use peaks::Peaks;

mod peaks;
mod ray;

/// Positions on the ground are `f32` metres snapped to sixteenths, which stay exact up to here.
pub const MAX_SIDE: u32 = 1 << 20;
pub const MIN_SIDE: u32 = 8;
/// The terrains one area's paint can name: a byte per half metre.
pub const MAX_TERRAINS: usize = 256;
pub const SAMPLES_PER_METER: u32 = 2;
/// The ground is cut into tiles this wide, a whole number of chunks, each a file of its own.
pub const TILE_CHUNKS: u32 = 8;
pub const TILE_SIDE: Meters = Meters(CHUNK_SIDE.0 * TILE_CHUNKS as f32);
/// The overview keeps every this-many-th value of the ground each way: what answers for a tile
/// not loaded.
pub const OVERVIEW_STRIDE: u32 = 8;

/// The pixels along a tile's side, and the samples but the one it shares with the next tile.
pub const TILE_PIXELS: u32 = TILE_SIDE.0 as u32 * SAMPLES_PER_METER;

/// A stretch of ground in whole metres, as an area's and a blueprint's canvas are measured.
#[derive(JsonSchema, Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroundSize {
    pub width: u32,
    pub height: u32,
}

impl GroundSize {
    pub fn grid(self) -> GridSize {
        GridSize::new(self.width, self.height)
    }

    pub fn meters(self) -> Size<Meters> {
        Size::new(self.width as f32, self.height as f32)
    }

    pub fn bounds(self) -> Rect<Meters> {
        self.meters().bounds()
    }

    /// Height samples across and down: one every half metre, both edges included.
    pub fn samples(self) -> (u32, u32) {
        (
            self.width * SAMPLES_PER_METER + 1,
            self.height * SAMPLES_PER_METER + 1,
        )
    }

    /// Paint pixels across and down: one every half metre.
    pub fn paint_pixels(self) -> (u32, u32) {
        (
            self.width * SAMPLES_PER_METER,
            self.height * SAMPLES_PER_METER,
        )
    }

    /// Tiles across and down, the last ones cut to the edge.
    pub fn tiles(self) -> (u32, u32) {
        let (columns, rows) = self.paint_pixels();
        (
            columns.div_ceil(TILE_PIXELS).max(1),
            rows.div_ceil(TILE_PIXELS).max(1),
        )
    }

    /// Every tile of the ground, row by row.
    pub fn each_tile(self) -> impl Iterator<Item = TileAt> {
        let (across, down) = self.tiles();
        (0..down).flat_map(move |row| (0..across).map(move |column| TileAt { column, row }))
    }

    /// The tile under `at`, the nearest one past the edge.
    pub fn tile_at(self, at: Pos<Meters>) -> TileAt {
        let (across, down) = self.tiles();
        let along = |value: f32, origin: f32| ((value - origin) / TILE_SIDE.0).floor().max(0.0);
        TileAt {
            column: (along(at.x, GROUND_ORIGIN.x) as u32).min(across - 1),
            row: (along(at.y, GROUND_ORIGIN.y) as u32).min(down - 1),
        }
    }

    /// Every tile holding any ground of `rect`, row by row.
    pub fn tiles_within(self, rect: Rect<Meters>) -> impl Iterator<Item = TileAt> {
        let low = self.tile_at(rect.origin);
        let high = self.tile_at(Pos::new(rect.max_x(), rect.max_y()));
        (low.row..=high.row).flat_map(move |row| {
            (low.column..=high.column).map(move |column| TileAt { column, row })
        })
    }

    pub fn check(self, pointer: &str, min_side: u32, cx: &mut RowCheck) {
        cx.range(
            format!("{pointer}/width"),
            self.width,
            min_side,
            MAX_SIDE,
            "m",
        );
        cx.range(
            format!("{pointer}/height"),
            self.height,
            min_side,
            MAX_SIDE,
            "m",
        );
    }
}

/// One tile of the ground, counted from its corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileAt {
    pub column: u32,
    pub row: u32,
}

impl TileAt {
    /// The tile's name in its folder of parts, `<column>_<row>`.
    pub fn key(self) -> String {
        format!("{}_{}", self.column, self.row)
    }

    pub fn from_key(key: &str) -> Option<TileAt> {
        let (column, row) = key.split_once('_')?;
        Some(TileAt {
            column: plain_number(column)?,
            row: plain_number(row)?,
        })
    }

    /// The tile holding the value at `(column, row)`.
    fn holding(column: u32, row: u32) -> TileAt {
        TileAt {
            column: column / TILE_PIXELS,
            row: row / TILE_PIXELS,
        }
    }

    /// The ground the tile covers, cut to a ground of `size`.
    pub fn rect(self, size: GroundSize) -> Rect<Meters> {
        let origin = Pos::new(
            GROUND_ORIGIN.x + self.column as f32 * TILE_SIDE.0,
            GROUND_ORIGIN.y + self.row as f32 * TILE_SIDE.0,
        );
        let bounds = size.bounds();
        Rect::new(
            origin,
            Size::new(
                TILE_SIDE.0.min(bounds.max_x() - origin.x).max(0.0),
                TILE_SIDE.0.min(bounds.max_y() - origin.y).max(0.0),
            ),
        )
    }
}

fn plain_number(text: &str) -> Option<u32> {
    let plain = text == "0" || (!text.starts_with('0') && !text.is_empty());
    (plain && text.bytes().all(|b| b.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()
}

/// The height of the ground at every half metre of an area, as its `height/` tiles hold it:
/// 16-bit grey across the area's [`HeightRange`]. Sample `(2x, 2y)` is the height at the centre
/// of the metre cell `(x, y)`. Tiles share the samples on their seams.
pub type Heightfield = GroundGrid<u16>;

/// The heights the 16 bits of an area's height tiles span, bottom level to top: wide enough for
/// its lowest and highest ground, as narrow as it allows for the finest steps. Ground level lies
/// on a step of its own, so flat ground stands at exactly nothing.
///
/// By default it holds ground level alone, as a ground not read yet does.
#[derive(JsonSchema, Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeightRange {
    pub lowest: Meters,
    pub highest: Meters,
}

impl HeightRange {
    /// Refuses a range that doesn't hold ground level, or holds no height but it.
    pub fn check(&self, pointer: &str, cx: &mut RowCheck) {
        if self.lowest.0 > 0.0 {
            cx.error(
                format!("{pointer}/lowest"),
                "above ground level",
                self.lowest.0,
                "meters at or below 0",
            );
        }
        if self.highest.0 < 0.0 {
            cx.error(
                format!("{pointer}/highest"),
                "below ground level",
                self.highest.0,
                "meters at or above 0",
            );
        }
        if self.highest.0 <= self.lowest.0 {
            cx.error(
                pointer,
                "empty range",
                format!("{} to {} m", self.lowest.0, self.highest.0),
                "a lowest height below the highest",
            );
        }
    }

    /// The metres between one level of the 16 bits and the next.
    pub fn step(self) -> f32 {
        (self.highest.0 - self.lowest.0) / u16::MAX as f32
    }

    /// The level ground level lies on.
    pub fn ground_level(self) -> u16 {
        (-self.lowest.0 / self.step()).round() as u16
    }

    fn level_of(self, height: Meters) -> u16 {
        (height.0 / self.step() + self.ground_level() as f32)
            .round()
            .clamp(0.0, u16::MAX as f32) as u16
    }

    fn height_of(self, level: u16) -> Meters {
        Meters((level as f32 - self.ground_level() as f32) * self.step())
    }
}

/// Which terrain paints every half metre of an area, as its `paint/` tiles hold it: a byte per
/// pixel, the index of the terrain in the area's list.
pub type PaintMap = GroundGrid<u8>;

/// The height of the water's surface at every half metre of an area, as its `water/` tiles hold
/// it, sampled and spanning the area's [`HeightRange`] as its heights do: water lies wherever its
/// surface stands above the ground, as deep as it stands above it, and the ground is dry wherever
/// it doesn't. With no tile, the whole area is dry.
pub type WaterSurface = GroundGrid<WaterLevel>;

impl GroundGrid<u16> {
    pub fn flat(size: GroundSize, range: HeightRange) -> Heightfield {
        GroundGrid::blank(TileGrid::samples(size), range)
    }
}

impl GroundGrid<WaterLevel> {
    /// No water anywhere over ground of `size`.
    pub fn dry(size: GroundSize, range: HeightRange) -> WaterSurface {
        GroundGrid::blank(TileGrid::samples(size), range)
    }

    /// How deep the water stands over `ground` at `at`: nothing where the ground is dry.
    pub fn depth_at(&self, ground: &Heightfield, at: Pos<Meters>) -> Meters {
        Meters((self.height_at(at).0 - ground.height_at(at).0).max(0.0))
    }
}

impl<T: Level> GroundGrid<T> {
    /// A grid of `size` over `range` whose sample `(column, row)` is `height(column, row)`.
    pub fn from_fn(
        size: GroundSize,
        range: HeightRange,
        height: impl Fn(u32, u32) -> Meters + Sync,
    ) -> GroundGrid<T> {
        GroundGrid::filled(TileGrid::samples(size), range, |column, row| {
            T::from_level(range.level_of(height(column, row)))
        })
    }

    /// The heights the samples span.
    pub fn range(&self) -> HeightRange {
        self.scale
    }

    /// Samples across and down.
    pub fn samples(&self) -> (u32, u32) {
        (self.grid.columns, self.grid.rows)
    }

    /// This grid over a terrain of `size`, kept from the top-left corner: what reaches past the
    /// new edge is cut, and new ground carries on at the old edge's height.
    pub fn resized(&self, size: GroundSize) -> GroundGrid<T> {
        self.load_all();
        GroundGrid::<T>::from_fn(size, self.scale, |column, row| self.sample(column, row))
    }

    /// Sets sample `(column, row)` to `height`, kept within the range the file holds.
    pub fn set_sample(&mut self, column: u32, row: u32, height: Meters) {
        let level = self.scale.level_of(height);
        self.put(column, row, T::from_level(level));
    }

    /// Where sample `(column, row)` lies on the ground.
    pub fn sample_pos(column: u32, row: u32) -> Pos<Meters> {
        let step = 1.0 / SAMPLES_PER_METER as f32;
        Pos::new(
            GROUND_ORIGIN.x + column as f32 * step,
            GROUND_ORIGIN.y + row as f32 * step,
        )
    }

    /// Sample `(column, row)`, the nearest past the edge; the overview's nearest where its tile
    /// isn't loaded.
    pub fn sample(&self, column: u32, row: u32) -> Meters {
        self.scale.height_of(self.index(column, row).level())
    }

    /// The ground's height at `at`, between the samples around it.
    pub fn height_at(&self, at: Pos<Meters>) -> Meters {
        let (x, y) = self.sample_space(at);
        let (column, row) = (x.floor(), y.floor());
        let (u, v) = (x - column, y - row);
        let (column, row) = (column.max(0.0) as u32, row.max(0.0) as u32);
        let lerp = |a: Meters, b: Meters, t: f32| Meters(a.0 + (b.0 - a.0) * t);
        let [top_left, top_right, bottom_left, bottom_right] = self
            .square(column, row)
            .map(|value| self.scale.height_of(value.level()));
        lerp(
            lerp(top_left, top_right, u),
            lerp(bottom_left, bottom_right, u),
            v,
        )
    }

    /// The steepest slope between neighbouring samples under each metre cell of the `size` cells
    /// from `origin`, row by row.
    pub fn steepest_in(&self, origin: CellPos, size: GridSize) -> Vec<Degrees> {
        let samples = self.window(
            (origin.x.max(0) as u32) * SAMPLES_PER_METER,
            (origin.y.max(0) as u32) * SAMPLES_PER_METER,
            size.width * SAMPLES_PER_METER + 1,
            size.height * SAMPLES_PER_METER + 1,
        );
        let step = 1.0 / SAMPLES_PER_METER as f32;
        let range = self.scale;
        let height = |column: u32, row: u32| range.height_of(samples.at(column, row).level()).0;
        size.cells()
            .map(|cell| {
                let (column, row) = (
                    cell.x as u32 * SAMPLES_PER_METER,
                    cell.y as u32 * SAMPLES_PER_METER,
                );
                let mut steepest = 0.0f32;
                for dy in 0..=SAMPLES_PER_METER {
                    for dx in 0..=SAMPLES_PER_METER {
                        let here = height(column + dx, row + dy);
                        if dx < SAMPLES_PER_METER {
                            let rise = (height(column + dx + 1, row + dy) - here).abs();
                            steepest = steepest.max(rise / step);
                        }
                        if dy < SAMPLES_PER_METER {
                            let rise = (height(column + dx, row + dy + 1) - here).abs();
                            steepest = steepest.max(rise / step);
                        }
                    }
                }
                Degrees(steepest.atan().to_degrees())
            })
            .collect()
    }

    fn sample_space(&self, at: Pos<Meters>) -> (f32, f32) {
        let x = ((at.x - GROUND_ORIGIN.x) * SAMPLES_PER_METER as f32)
            .clamp(0.0, self.grid.columns.saturating_sub(1) as f32);
        let y = ((at.y - GROUND_ORIGIN.y) * SAMPLES_PER_METER as f32)
            .clamp(0.0, self.grid.rows.saturating_sub(1) as f32);
        (x, y)
    }
}

impl GroundGrid<u8> {
    /// All the first terrain.
    pub fn plain(size: GroundSize) -> PaintMap {
        GroundGrid::blank(TileGrid::pixels(size), ())
    }

    /// A paint of `size` whose pixel `(column, row)` is the terrain `index(column, row)`.
    pub fn from_fn(size: GroundSize, index: impl Fn(u32, u32) -> u8 + Sync) -> PaintMap {
        GroundGrid::filled(TileGrid::pixels(size), (), index)
    }

    /// Pixels across and down.
    pub fn pixels(&self) -> (u32, u32) {
        (self.grid.columns, self.grid.rows)
    }

    /// This paint over a ground of `size`, kept from the top-left corner: what reaches past the
    /// new edge is cut, and new ground is the first terrain.
    pub fn resized(&self, size: GroundSize) -> PaintMap {
        self.load_all();
        let (columns, rows) = self.pixels();
        PaintMap::from_fn(size, |column, row| {
            if column < columns && row < rows {
                self.index(column, row)
            } else {
                0
            }
        })
    }

    /// Paints pixel `(column, row)` with the terrain `index`.
    pub fn set(&mut self, column: u32, row: u32, index: u8) {
        self.put(column, row, index);
    }

    /// The middle of pixel `(column, row)` on the ground.
    pub fn pixel_pos(column: u32, row: u32) -> Pos<Meters> {
        let step = 1.0 / SAMPLES_PER_METER as f32;
        Pos::new(
            GROUND_ORIGIN.x + (column as f32 + 0.5) * step,
            GROUND_ORIGIN.y + (row as f32 + 0.5) * step,
        )
    }

    /// The terrain painting the half metre under `at`.
    pub fn at(&self, at: Pos<Meters>) -> u8 {
        let (column, row) = self.pixel_at(at);
        self.index(column, row)
    }

    /// The highest terrain index any pixel names, every tile loaded.
    pub fn highest(&self) -> u8 {
        self.load_all();
        self.tiles
            .iter()
            .filter_map(|slot| slot.tile()?.values.iter().copied().max())
            .max()
            .unwrap_or(0)
    }

    /// The terrain painting most of the metre cell `cell`; of equals, the first.
    pub fn dominant(&self, cell: CellPos) -> u8 {
        dominant_of(cell, |column, row| self.index(column, row))
    }

    /// The terrain painting most of each metre cell of the `size` cells from `origin`, row by
    /// row; of equals, the first.
    pub fn dominant_in(&self, origin: CellPos, size: GridSize) -> Vec<u8> {
        let pixels = self.window(
            (origin.x.max(0) as u32) * SAMPLES_PER_METER,
            (origin.y.max(0) as u32) * SAMPLES_PER_METER,
            size.width * SAMPLES_PER_METER,
            size.height * SAMPLES_PER_METER,
        );
        size.cells()
            .map(|cell| dominant_of(cell, |column, row| pixels.at(column, row)))
            .collect()
    }

    /// The pixel under `at`.
    pub fn pixel_at(&self, at: Pos<Meters>) -> (u32, u32) {
        let column = ((at.x - GROUND_ORIGIN.x) * SAMPLES_PER_METER as f32).floor();
        let row = ((at.y - GROUND_ORIGIN.y) * SAMPLES_PER_METER as f32).floor();
        (
            (column.max(0.0) as u32).min(self.grid.columns.saturating_sub(1)),
            (row.max(0.0) as u32).min(self.grid.rows.saturating_sub(1)),
        )
    }
}

fn dominant_of(cell: CellPos, pixel: impl Fn(u32, u32) -> u8) -> u8 {
    let (column, row) = (
        cell.x as u32 * SAMPLES_PER_METER,
        cell.y as u32 * SAMPLES_PER_METER,
    );
    let mut counts = [0u32; MAX_TERRAINS];
    for dy in 0..SAMPLES_PER_METER {
        for dx in 0..SAMPLES_PER_METER {
            counts[usize::from(pixel(column + dx, row + dy))] += 1;
        }
    }
    counts
        .iter()
        .enumerate()
        .rev()
        .max_by_key(|(_, count)| **count)
        .map_or(0, |(index, _)| index as u8)
}

/// The values a grid over the ground can hold, kept out of reach so the ground's grids are the
/// only ones.
mod values {
    use super::HeightRange;

    /// A value that is a height over its grid's [`HeightRange`], one of the 16-bit steps across it.
    pub trait Level: Value<Scale = HeightRange> {
        fn level(self) -> u16;
        fn from_level(level: u16) -> Self;
    }

    impl Level for u16 {
        fn level(self) -> u16 {
            self
        }

        fn from_level(level: u16) -> u16 {
            level
        }
    }

    /// The height of the water's surface over the ground's [`HeightRange`], as a ground height is,
    /// at the bottom of the range where nothing says otherwise, so ground without water is dry.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
    pub struct WaterLevel(pub(super) u16);

    impl Level for WaterLevel {
        fn level(self) -> u16 {
            self.0
        }

        fn from_level(level: u16) -> WaterLevel {
            WaterLevel(level)
        }
    }

    impl Value for WaterLevel {
        type Scale = HeightRange;
        const CHANNELS: Channels = Channels::Grey16;

        fn blank(_: HeightRange) -> WaterLevel {
            WaterLevel(0)
        }

        fn decode(bytes: Vec<u8>) -> Vec<WaterLevel> {
            u16::decode(bytes).into_iter().map(WaterLevel).collect()
        }

        fn encode(values: &[WaterLevel]) -> Vec<u8> {
            values
                .iter()
                .flat_map(|level| level.0.to_be_bytes())
                .collect()
        }
    }

    /// A value of the ground as its files hold it.
    pub trait Value: Copy + Ord + Send + Sync + std::fmt::Debug + 'static {
        /// What a grid's values stand for, beside the values.
        type Scale: Copy + Default + PartialEq + Send + Sync + std::fmt::Debug + 'static;
        const CHANNELS: Channels;
        /// What the ground is where no file says otherwise.
        fn blank(scale: Self::Scale) -> Self;
        fn decode(bytes: Vec<u8>) -> Vec<Self>;
        fn encode(values: &[Self]) -> Vec<u8>;
    }

    impl Value for u16 {
        type Scale = HeightRange;
        const CHANNELS: Channels = Channels::Grey16;

        fn blank(range: HeightRange) -> u16 {
            range.ground_level()
        }

        fn decode(bytes: Vec<u8>) -> Vec<u16> {
            bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_be_bytes(*pair))
                .collect()
        }

        fn encode(values: &[u16]) -> Vec<u8> {
            values
                .iter()
                .flat_map(|level| level.to_be_bytes())
                .collect()
        }
    }

    impl Value for u8 {
        type Scale = ();
        const CHANNELS: Channels = Channels::Grey8;

        fn blank(_: ()) -> u8 {
            0
        }

        fn decode(bytes: Vec<u8>) -> Vec<u8> {
            bytes
        }

        fn encode(values: &[u8]) -> Vec<u8> {
            values.to_vec()
        }
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    pub enum Channels {
        Grey16,
        Grey8,
    }
}

pub use values::{Level, Value as GroundValue, WaterLevel};

/// Which values a tile of the ground holds, as [`GroundGrid::version`] tells it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TileVersion(u64);

fn next_revision() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// A share, 0 to 1, of something over the whole of a ground, as coarse as its maker likes: a byte
/// a pixel, the pixels spread evenly over the ground whatever its size, read between them. An
/// area's rain map is one, the share of the downpour that falls on each stretch of it.
#[derive(Clone, Debug, PartialEq)]
pub struct ShareMap {
    columns: u32,
    rows: u32,
    shares: Vec<u8>,
}

impl ShareMap {
    /// The map `columns` × `rows` pixels, each the share `share` says of the pixel's centre in
    /// the ground's own measure, 0 to 1 each way.
    pub fn from_fn(columns: u32, rows: u32, share: impl Fn(f32, f32) -> f32) -> ShareMap {
        let columns = columns.max(1);
        let rows = rows.max(1);
        let shares = (0..rows)
            .flat_map(|row| (0..columns).map(move |column| (column, row)))
            .map(|(column, row)| {
                let across = (column as f32 + 0.5) / columns as f32;
                let down = (row as f32 + 0.5) / rows as f32;
                (share(across, down).clamp(0.0, 1.0) * 255.0).round() as u8
            })
            .collect();
        ShareMap {
            columns,
            rows,
            shares,
        }
    }

    pub fn columns(&self) -> u32 {
        self.columns
    }

    pub fn rows(&self) -> u32 {
        self.rows
    }

    /// The share at `at` on a ground spanning `bounds`, read between the pixels round it and held
    /// at the edge's past the ground.
    pub fn share_at(&self, bounds: Rect<Meters>, at: Pos<Meters>) -> f32 {
        let x = (at.x - bounds.origin.x) / bounds.size.width.max(f32::EPSILON)
            * self.columns as f32
            - 0.5;
        let y = (at.y - bounds.origin.y) / bounds.size.height.max(f32::EPSILON) * self.rows as f32
            - 0.5;
        let (left, top) = (x.floor(), y.floor());
        let (tx, ty) = (x - left, y - top);
        let pixel = |column: f32, row: f32| {
            let column = (column as i64).clamp(0, i64::from(self.columns) - 1) as usize;
            let row = (row as i64).clamp(0, i64::from(self.rows) - 1) as usize;
            f32::from(self.shares[row * self.columns as usize + column]) / 255.0
        };
        let upper = pixel(left, top) * (1.0 - tx) + pixel(left + 1.0, top) * tx;
        let lower = pixel(left, top + 1.0) * (1.0 - tx) + pixel(left + 1.0, top + 1.0) * tx;
        upper * (1.0 - ty) + lower * ty
    }

    /// The map of a ground of `from` metres as it lies over a ground of `to`, kept from the
    /// top-left corner at its own pixels to the metre, the ground it gains taking the share at
    /// its edge.
    pub fn resized(&self, from: Size<Meters>, to: Size<Meters>) -> ShareMap {
        let per_metre_x = self.columns as f32 / from.width.max(f32::EPSILON);
        let per_metre_y = self.rows as f32 / from.height.max(f32::EPSILON);
        let columns = (to.width * per_metre_x).round().max(1.0) as u32;
        let rows = (to.height * per_metre_y).round().max(1.0) as u32;
        let shares = (0..rows)
            .flat_map(|row| (0..columns).map(move |column| (column, row)))
            .map(|(column, row)| {
                let column = column.min(self.columns - 1) as usize;
                let row = row.min(self.rows - 1) as usize;
                self.shares[row * self.columns as usize + column]
            })
            .collect();
        ShareMap {
            columns,
            rows,
            shares,
        }
    }

    /// The map an 8-bit greyscale PNG of any size holds.
    pub fn decode(bytes: &[u8]) -> Result<ShareMap, ContentProblem> {
        let ((columns, rows), shares) = read_png(bytes, Channels::Grey8, None)?;
        if columns == 0 || rows == 0 {
            return Err(ContentProblem::new(
                "empty image",
                "0 px",
                "a pixel or more each way",
            ));
        }
        Ok(ShareMap {
            columns,
            rows,
            shares,
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        encode_png(&self.shares, self.columns, self.rows, Channels::Grey8)
    }
}
use values::{Channels, Value};

/// A grid of values over the ground, held tile by tile with an overview of the whole. Copies
/// share their tiles, so one copy's loading serves them all, and a copy that sets a value takes a
/// tile of its own first.
#[derive(Clone, Debug)]
pub struct GroundGrid<T: Value> {
    grid: TileGrid,
    scale: T::Scale,
    overview: Option<Arc<Tile<T>>>,
    tiles: Vec<Arc<Slot<T>>>,
}

impl<T: Value> Default for GroundGrid<T> {
    fn default() -> GroundGrid<T> {
        GroundGrid {
            grid: TileGrid::default(),
            scale: T::Scale::default(),
            overview: None,
            tiles: Vec::new(),
        }
    }
}

/// Reads a grid's values as [`GroundGrid::index`] does, keeping hold of the tile it read last, so
/// reads that fall in one tile look it up once.
pub struct GroundReader<'a, T: Value> {
    ground: &'a GroundGrid<T>,
    held: Option<(TileAt, Option<Arc<Tile<T>>>)>,
}

impl<T: Value> GroundReader<'_, T> {
    pub fn index(&mut self, column: u32, row: u32) -> T {
        let ground = self.ground;
        let column = column.min(ground.grid.columns.saturating_sub(1));
        let row = row.min(ground.grid.rows.saturating_sub(1));
        let at = TileAt::holding(column, row);
        let tile = match &self.held {
            Some((holding, tile)) if *holding == at => tile,
            _ => {
                let tile = ground
                    .tiles
                    .get(ground.grid.index(at))
                    .and_then(|slot| slot.tile());
                &self.held.insert((at, tile)).1
            }
        };
        match tile {
            Some(tile) => tile.values[tile.offset(ground.grid.first(at), column, row)],
            None => ground.overview_value(column, row),
        }
    }
}

impl<T: Level> GroundReader<'_, T> {
    /// Sample `(column, row)`, as [`GroundGrid::sample`] reads it.
    pub fn sample(&mut self, column: u32, row: u32) -> Meters {
        let level = self.index(column, row).level();
        self.ground.scale.height_of(level)
    }
}

/// A window of values read out of the tiles, row by row.
struct Window<T> {
    columns: u32,
    values: Vec<T>,
}

impl<T: Value> Window<T> {
    fn at(&self, column: u32, row: u32) -> T {
        self.values[(row * self.columns + column) as usize]
    }
}

/// One tile's values row by row, with the file they came from while they are as it holds them.
#[derive(Clone, Debug)]
struct Tile<T> {
    columns: u32,
    rows: u32,
    values: Vec<T>,
    peaks: OnceLock<Peaks<T>>,
    encoded: Option<Arc<[u8]>>,
    /// Which values these are: no other tile, nor this one once a value changes, has the same.
    revision: u64,
}

impl<T> Tile<T> {
    /// Where among the values of the tile beginning at `first` the value at `(column, row)` lies.
    fn offset(&self, first: (u32, u32), column: u32, row: u32) -> usize {
        ((row - first.1) * self.columns + (column - first.0)) as usize
    }
}

/// A tile of the ground as it is held: its values once loaded, else the file to read them from,
/// or nothing where the ground is blank. `loading` is held while the file is read, so callers
/// asking for the tile at once wait for the one reading rather than read it again.
#[derive(Debug)]
struct Slot<T> {
    loaded: RwLock<Option<Arc<Tile<T>>>>,
    loading: Mutex<()>,
    stored: Option<StoredPart>,
}

impl<T: Value> Slot<T> {
    fn new(loaded: Option<Arc<Tile<T>>>, stored: Option<StoredPart>) -> Slot<T> {
        Slot {
            loaded: RwLock::new(loaded),
            loading: Mutex::new(()),
            stored,
        }
    }

    fn tile(&self) -> Option<Arc<Tile<T>>> {
        self.loaded.read().expect("a ground tile lock").clone()
    }

    fn is_loaded(&self) -> bool {
        self.stored.is_none() || self.loaded.read().expect("a ground tile lock").is_some()
    }
}

impl<T: Value> GroundGrid<T> {
    fn blank(grid: TileGrid, scale: T::Scale) -> GroundGrid<T> {
        GroundGrid {
            tiles: (0..grid.count())
                .map(|_| Arc::new(Slot::new(None, None)))
                .collect(),
            grid,
            scale,
            overview: None,
        }
    }

    fn filled(
        grid: TileGrid,
        scale: T::Scale,
        value: impl Fn(u32, u32) -> T + Sync,
    ) -> GroundGrid<T> {
        let tiles = (0..grid.count())
            .into_par_iter()
            .map(|index| {
                let at = grid.at(index);
                let (first_column, first_row) = grid.first(at);
                let (columns, rows) = grid.tile_dims(at);
                let values = (0..rows)
                    .flat_map(|row| (0..columns).map(move |column| (column, row)))
                    .map(|(column, row)| value(first_column + column, first_row + row))
                    .collect();
                let tile = Tile {
                    columns,
                    rows,
                    values,
                    peaks: OnceLock::new(),
                    revision: next_revision(),
                    encoded: None,
                };
                Arc::new(Slot::new(Some(Arc::new(tile)), None))
            })
            .collect();
        GroundGrid {
            grid,
            scale,
            overview: None,
            tiles,
        }
    }

    pub fn size(&self) -> GroundSize {
        self.grid.size
    }

    fn blank_value(&self) -> T {
        T::blank(self.scale)
    }

    /// Keeps `part` as the tile at `at`, to read when the tile is first loaded.
    pub fn store(&mut self, at: TileAt, part: StoredPart) {
        self.tiles[self.grid.index(at)] = Arc::new(Slot::new(None, Some(part)));
    }

    /// Reads the overview from its PNG, refusing one that isn't the ground's.
    pub fn decode_overview(&mut self, bytes: &[u8]) -> Result<(), ContentProblem> {
        let (columns, rows) = self.grid.overview_dims();
        let pixels = decode_png(bytes, columns, rows, T::CHANNELS)?;
        self.overview = Some(Arc::new(Tile {
            columns,
            rows,
            values: T::decode(pixels),
            peaks: OnceLock::new(),
            revision: next_revision(),
            encoded: Some(Arc::from(bytes)),
        }));
        Ok(())
    }

    /// Whether the tile at `at` is loaded, so its values answer exactly rather than from the
    /// overview.
    pub fn is_loaded(&self, at: TileAt) -> bool {
        self.tiles[self.grid.index(at)].is_loaded()
    }

    /// Which values the tile at `at` holds now: the same version only while they are the same
    /// values, none for a tile not loaded, blank or past the ground.
    pub fn version(&self, at: TileAt) -> TileVersion {
        let inside = at.column < self.grid.across && at.row < self.grid.down;
        TileVersion(
            inside
                .then(|| self.tiles[self.grid.index(at)].tile())
                .flatten()
                .map_or(0, |tile| tile.revision),
        )
    }

    /// The `columns` × `rows` values from `first`, row by row, which may begin before the grid
    /// does: each as [`GroundGrid::index`] gives it, the nearest past every edge.
    pub fn block(&self, first: (i64, i64), columns: u32, rows: u32) -> Vec<T> {
        let start = (first.0.max(0) as u32, first.1.max(0) as u32);
        let lead = (
            (i64::from(start.0) - first.0) as u32,
            (i64::from(start.1) - first.1) as u32,
        );
        let inner_columns = columns.saturating_sub(lead.0).max(1);
        let inner_rows = rows.saturating_sub(lead.1).max(1);
        let inner = self.window(start.0, start.1, inner_columns, inner_rows);
        let mut values = Vec::with_capacity((columns * rows) as usize);
        for row in 0..rows {
            let row = row.saturating_sub(lead.1).min(inner_rows - 1);
            for column in 0..columns {
                values.push(inner.at(column.saturating_sub(lead.0).min(inner_columns - 1), row));
            }
        }
        values
    }

    /// Overview values across and down.
    pub fn overview_dims(&self) -> (u32, u32) {
        self.grid.overview_dims()
    }

    /// The overview values at each of `columns` in each of `rows`, row by row, as the tiles hold
    /// them now: each read from its tile where that is loaded, so they follow its edits.
    pub fn overview_at(&self, columns: &[u32], rows: &[u32]) -> Vec<T> {
        let mut reader = self.reader();
        let mut values = Vec::with_capacity(columns.len() * rows.len());
        for &row in rows {
            for &column in columns {
                let (column, row) = self.grid.overview_source(column, row);
                values.push(reader.index(column, row));
            }
        }
        values
    }

    /// The overview values the tile at `at` holds, as their first and how many across and down.
    pub fn overview_span(&self, at: TileAt) -> ((u32, u32), u32, u32) {
        let (first_column, first_row) = self.grid.first(at);
        let (columns, rows) = self.grid.tile_dims(at);
        let (across, down) = self.grid.overview_dims();
        let (low_column, high_column) = self.grid.overview_within(
            first_column,
            first_column + columns,
            self.grid.columns,
            across,
        );
        let (low_row, high_row) =
            self.grid
                .overview_within(first_row, first_row + rows, self.grid.rows, down);
        (
            (low_column, low_row),
            high_column.saturating_sub(low_column),
            high_row.saturating_sub(low_row),
        )
    }

    /// Loads the tile at `at` from its file, unless it is loaded or has none; refuses a file that
    /// isn't the tile's picture, leaving the tile as it was.
    pub fn load(&self, at: TileAt) -> Result<(), ContentProblem> {
        let slot = &self.tiles[self.grid.index(at)];
        let Some(stored) = &slot.stored else {
            return Ok(());
        };
        if slot.is_loaded() {
            return Ok(());
        }
        let _reading = slot
            .loading
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if slot.is_loaded() {
            return Ok(());
        }
        let bytes = stored.read().map_err(|error| {
            ContentProblem::new("unreadable file", error.to_string(), "a readable file")
        })?;
        let mut tile = self.grid.decode_tile(at, &bytes)?;
        tile.encoded = None;
        *slot.loaded.write().expect("a ground tile lock") = Some(Arc::new(tile));
        Ok(())
    }

    /// Loads every tile, on every core: the problems of the tiles that don't load.
    pub fn load_all(&self) -> Vec<(TileAt, ContentProblem)> {
        (0..self.tiles.len())
            .into_par_iter()
            .filter_map(|index| {
                let at = self.grid.at(index);
                self.load(at).err().map(|problem| (at, problem))
            })
            .collect()
    }

    /// Whether every value is blank, without loading: a tile with a file to load counts as not.
    fn is_blank(&self) -> bool {
        self.tiles.iter().all(|slot| match slot.tile() {
            Some(tile) => tile.is_blank(self.blank_value()),
            None => slot.stored.is_none(),
        })
    }

    /// The tile at `at` as its PNG, none for a blank tile, which no file holds.
    pub fn encode_tile(&self, at: TileAt) -> Option<Arc<[u8]>> {
        let slot = &self.tiles[self.grid.index(at)];
        if let Some(stored) = &slot.stored {
            return stored.read().ok().map(Arc::from);
        }
        let tile = slot.tile()?;
        if tile.is_blank(self.blank_value()) {
            return None;
        }
        Some(tile.png())
    }

    /// The overview the tiles make as its PNG, none when the ground is blank.
    pub fn encode_overview(&self) -> Option<Vec<u8>> {
        let overview = self.overview_of_tiles()?;
        if let Some(held) = &self.overview
            && held.values == overview.values
            && let Some(bytes) = &held.encoded
        {
            return Some(bytes.to_vec());
        }
        Some(encode_png(
            &T::encode(&overview.values),
            overview.columns,
            overview.rows,
            T::CHANNELS,
        ))
    }

    /// Whether the overview read says what the tiles say.
    pub fn overview_is_current(&self) -> bool {
        match (self.overview_of_tiles(), &self.overview) {
            (None, None) => true,
            (Some(made), Some(held)) => made.values == held.values,
            _ => false,
        }
    }

    /// The overview the tiles make, every tile loaded; none when the ground is blank.
    fn overview_of_tiles(&self) -> Option<Tile<T>> {
        self.load_all();
        if self.is_blank() {
            return None;
        }
        let (columns, rows) = self.grid.overview_dims();
        let values = (0..rows)
            .flat_map(|row| (0..columns).map(move |column| (column, row)))
            .map(|(column, row)| {
                let (column, row) = self.grid.overview_source(column, row);
                self.index(column, row)
            })
            .collect();
        Some(Tile {
            columns,
            rows,
            values,
            peaks: OnceLock::new(),
            revision: next_revision(),
            encoded: None,
        })
    }

    /// The `columns` × `rows` values from `(column, row)`, row by row, each as [`GroundGrid::index`]
    /// gives it, read a tile at a time.
    fn window(&self, column: u32, row: u32, columns: u32, rows: u32) -> Window<T> {
        let mut values = Vec::with_capacity((columns * rows) as usize);
        let last_column = self.grid.columns.saturating_sub(1);
        let last_row = self.grid.rows.saturating_sub(1);
        let end = column + columns;
        let mut held: Option<(TileAt, Option<Arc<Tile<T>>>)> = None;
        for row in row..row + rows {
            let row = row.min(last_row);
            let mut from = column;
            while from < end {
                let clamped = from.min(last_column);
                let at = TileAt::holding(clamped, row);
                let first = self.grid.first(at);
                let past_edge = from > last_column;
                let until = if past_edge {
                    end
                } else {
                    end.min(first.0 + TILE_PIXELS).min(last_column + 1)
                };
                let tile = match &held {
                    Some((holding, tile)) if *holding == at => tile,
                    _ => {
                        let tile = self
                            .tiles
                            .get(self.grid.index(at))
                            .and_then(|slot| slot.tile());
                        &held.insert((at, tile)).1
                    }
                };
                let run = (until - from) as usize;
                match tile {
                    Some(tile) => {
                        let start = tile.offset(first, clamped, row);
                        if past_edge {
                            values.extend(std::iter::repeat_n(tile.values[start], run));
                        } else {
                            values.extend_from_slice(&tile.values[start..start + run]);
                        }
                    }
                    None => values.extend(
                        (from..until)
                            .map(|column| self.overview_value(column.min(last_column), row)),
                    ),
                }
                from = until;
            }
        }
        Window { columns, values }
    }

    /// The value at `(column, row)`, the nearest one past the edge: from its tile when that is
    /// loaded, else the overview's nearest, else blank.
    pub fn index(&self, column: u32, row: u32) -> T {
        let column = column.min(self.grid.columns.saturating_sub(1));
        let row = row.min(self.grid.rows.saturating_sub(1));
        let at = TileAt::holding(column, row);
        let Some(slot) = self.tiles.get(self.grid.index(at)) else {
            return self.blank_value();
        };
        if let Some(tile) = slot.loaded.read().expect("a ground tile lock").as_ref() {
            return tile.values[tile.offset(self.grid.first(at), column, row)];
        }
        self.overview_value(column, row)
    }

    /// A reader of values near one another, for many reads at once.
    pub fn reader(&self) -> GroundReader<'_, T> {
        GroundReader {
            ground: self,
            held: None,
        }
    }

    /// The values at `(column, row)` and at its neighbours to the right, below and across, each the
    /// nearest past the edge: read from one tile at once where all four lie in it.
    fn square(&self, column: u32, row: u32) -> [T; 4] {
        let last_column = self.grid.columns.saturating_sub(1);
        let last_row = self.grid.rows.saturating_sub(1);
        let (column, row) = (column.min(last_column), row.min(last_row));
        let (right, below) = ((column + 1).min(last_column), (row + 1).min(last_row));
        let at = TileAt::holding(column, row);
        if TileAt::holding(right, below) == at
            && let Some(slot) = self.tiles.get(self.grid.index(at))
            && let Some(tile) = slot.loaded.read().expect("a ground tile lock").as_ref()
        {
            let first = self.grid.first(at);
            let value = |column: u32, row: u32| tile.values[tile.offset(first, column, row)];
            return [
                value(column, row),
                value(right, row),
                value(column, below),
                value(right, below),
            ];
        }
        [
            self.index(column, row),
            self.index(right, row),
            self.index(column, below),
            self.index(right, below),
        ]
    }

    /// The overview's value nearest `(column, row)`, blank without one.
    fn overview_value(&self, column: u32, row: u32) -> T {
        match &self.overview {
            Some(overview) => {
                let (column, row) = self.grid.overview_nearest(column, row);
                overview.values[(row * overview.columns + column) as usize]
            }
            None => self.blank_value(),
        }
    }

    /// Sets the value at `(column, row)` in every tile holding it, loading a tile first and
    /// taking a tile of this copy's own where one is shared with another.
    fn put(&mut self, column: u32, row: u32, value: T) {
        let grid = self.grid;
        let blank = self.blank_value();
        for (index, local) in grid.holders(column, row) {
            let at = grid.at(index);
            if let Err(problem) = self.load(at) {
                warn!(
                    "ground tile {} doesn't load, and is set anew: {problem}",
                    at.key()
                );
            }
            let slot = &mut self.tiles[index];
            let unchanged = match slot.tile() {
                Some(tile) => tile.values[local] == value,
                None => value == blank,
            };
            if unchanged {
                continue;
            }
            if Arc::get_mut(slot).is_none() {
                *slot = Arc::new(Slot::new(slot.tile(), None));
            }
            let slot = Arc::get_mut(slot).expect("a slot of this copy's own");
            slot.stored = None;
            let loaded = slot.loaded.get_mut().expect("a ground tile lock");
            let tile = loaded.get_or_insert_with(|| {
                let (columns, rows) = grid.tile_dims(at);
                Arc::new(Tile {
                    columns,
                    rows,
                    values: vec![blank; (columns * rows) as usize],
                    peaks: OnceLock::new(),
                    revision: next_revision(),
                    encoded: None,
                })
            });
            let tile = Arc::make_mut(tile);
            tile.encoded = None;
            tile.revision = next_revision();
            tile.values[local] = value;
            if let Some(peaks) = tile.peaks.get_mut() {
                let local = local as u32;
                peaks.raise(local % tile.columns, local / tile.columns, value);
            }
        }
    }
}

impl<T: Value> PartialEq for GroundGrid<T> {
    fn eq(&self, other: &GroundGrid<T>) -> bool {
        if self.grid != other.grid || self.scale != other.scale {
            return false;
        }
        self.load_all();
        other.load_all();
        self.tiles
            .iter()
            .zip(&other.tiles)
            .all(|(a, b)| match (a.tile(), b.tile()) {
                (Some(a), Some(b)) => Arc::ptr_eq(&a, &b) || a.values == b.values,
                (Some(tile), None) | (None, Some(tile)) => tile.is_blank(self.blank_value()),
                (None, None) => true,
            })
    }
}

impl<T: Value> Tile<T> {
    /// At least the highest of the tile's values from `low` to `high`, both included.
    fn highest(&self, low: (u32, u32), high: (u32, u32)) -> T {
        self.peaks
            .get_or_init(|| Peaks::of(self.columns, self.rows, &self.values))
            .highest(&self.values, low, high)
    }

    fn is_blank(&self, blank: T) -> bool {
        self.values.iter().all(|value| *value == blank)
    }

    /// The tile's PNG: the file it came from while it is as it holds it, else encoded now.
    fn png(&self) -> Arc<[u8]> {
        match &self.encoded {
            Some(bytes) => bytes.clone(),
            None => Arc::from(encode_png(
                &T::encode(&self.values),
                self.columns,
                self.rows,
                T::CHANNELS,
            )),
        }
    }
}

/// How a grid of values over the ground is cut into tiles: `columns`×`rows` values in all, each
/// tile `TILE_PIXELS` across but the last ones, which end at the edge; a grid of samples has one
/// value more than a grid of pixels each way, and its tiles share the values on their seams.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct TileGrid {
    size: GroundSize,
    columns: u32,
    rows: u32,
    across: u32,
    down: u32,
    /// How many values a tile holds past the pixels of its ground: one for samples, none for
    /// pixels.
    seam: u32,
}

impl TileGrid {
    fn samples(size: GroundSize) -> TileGrid {
        let (columns, rows) = size.samples();
        let (across, down) = size.tiles();
        TileGrid {
            size,
            columns,
            rows,
            across,
            down,
            seam: 1,
        }
    }

    fn pixels(size: GroundSize) -> TileGrid {
        let (columns, rows) = size.paint_pixels();
        let (across, down) = size.tiles();
        TileGrid {
            size,
            columns,
            rows,
            across,
            down,
            seam: 0,
        }
    }

    fn count(&self) -> usize {
        (self.across * self.down) as usize
    }

    fn index(&self, at: TileAt) -> usize {
        (at.row * self.across + at.column) as usize
    }

    fn at(&self, index: usize) -> TileAt {
        TileAt {
            column: index as u32 % self.across,
            row: index as u32 / self.across,
        }
    }

    /// The first value of the tile at `at` in the grid.
    fn first(&self, at: TileAt) -> (u32, u32) {
        (at.column * TILE_PIXELS, at.row * TILE_PIXELS)
    }

    fn tile_dims(&self, at: TileAt) -> (u32, u32) {
        let (first_column, first_row) = self.first(at);
        (
            (self.columns - first_column).min(TILE_PIXELS + self.seam),
            (self.rows - first_row).min(TILE_PIXELS + self.seam),
        )
    }

    fn decode_tile<T: Value>(&self, at: TileAt, bytes: &[u8]) -> Result<Tile<T>, ContentProblem> {
        let (columns, rows) = self.tile_dims(at);
        let pixels = decode_png(bytes, columns, rows, T::CHANNELS)?;
        Ok(Tile {
            columns,
            rows,
            values: T::decode(pixels),
            peaks: OnceLock::new(),
            revision: next_revision(),
            encoded: Some(Arc::from(bytes)),
        })
    }

    /// Overview values across and down: one per [`OVERVIEW_STRIDE`] of the grid's, and for a grid
    /// of samples the one at the far edge too.
    fn overview_dims(&self) -> (u32, u32) {
        let along = |values: u32| {
            if self.seam > 0 {
                (values - 1).div_ceil(OVERVIEW_STRIDE) + 1
            } else {
                values.div_ceil(OVERVIEW_STRIDE)
            }
        };
        (along(self.columns), along(self.rows))
    }

    /// The value of the grid the overview value `(column, row)` is: a sample on the stride, the
    /// last one at the far edge; a pixel in the middle of its block of the stride.
    fn overview_source(&self, column: u32, row: u32) -> (u32, u32) {
        let middle = if self.seam > 0 {
            0
        } else {
            OVERVIEW_STRIDE / 2
        };
        (
            (column * OVERVIEW_STRIDE + middle).min(self.columns - 1),
            (row * OVERVIEW_STRIDE + middle).min(self.rows - 1),
        )
    }

    /// The overview values along one way whose values lie from `start` up to `end`, of `values`
    /// along it and `count` overview values: the first of them and the one past the last.
    fn overview_within(&self, start: u32, end: u32, values: u32, count: u32) -> (u32, u32) {
        let middle = if self.seam > 0 {
            0
        } else {
            OVERVIEW_STRIDE / 2
        };
        let low = start
            .saturating_sub(middle)
            .div_ceil(OVERVIEW_STRIDE)
            .min(count);
        let high = if end >= values {
            count
        } else {
            (end.saturating_sub(1 + middle) / OVERVIEW_STRIDE + 1).min(count)
        };
        (low, high.max(low))
    }

    /// The overview value nearest the grid's `(column, row)`.
    fn overview_nearest(&self, column: u32, row: u32) -> (u32, u32) {
        let (across, down) = self.overview_dims();
        let nearest = |value: u32, count: u32, values: u32| {
            let lower = (value / OVERVIEW_STRIDE).min(count - 1);
            let upper = (lower + 1).min(count - 1);
            let source = |overview: u32| (overview * OVERVIEW_STRIDE).min(values - 1);
            if self.seam > 0 && value - source(lower) > source(upper) - value {
                upper
            } else {
                lower
            }
        };
        (
            nearest(column, across, self.columns),
            nearest(row, down, self.rows),
        )
    }

    /// Every tile holding the value at `(column, row)`, with the value's place in it: the tile it
    /// falls in, and the one before it along each seam it lies on.
    fn holders(&self, column: u32, row: u32) -> Vec<(usize, usize)> {
        if column >= self.columns || row >= self.rows {
            return Vec::new();
        }
        let along = |value: u32| {
            let own = value / TILE_PIXELS;
            let before =
                (self.seam > 0 && value.is_multiple_of(TILE_PIXELS) && own > 0).then(|| own - 1);
            [Some(own), before]
        };
        let mut holders = Vec::with_capacity(4);
        for tile_column in along(column).into_iter().flatten() {
            for tile_row in along(row).into_iter().flatten() {
                let at = TileAt {
                    column: tile_column,
                    row: tile_row,
                };
                let (first_column, first_row) = self.first(at);
                let (columns, _) = self.tile_dims(at);
                let local = ((row - first_row) * columns + (column - first_column)) as usize;
                holders.push((self.index(at), local));
            }
        }
        holders
    }
}

/// The greyscale PNG `bytes` hold, as its raw pixel bytes: refused unless it is `columns` ×
/// `rows` of `channels`.
fn decode_png(
    bytes: &[u8],
    columns: u32,
    rows: u32,
    channels: Channels,
) -> Result<Vec<u8>, ContentProblem> {
    read_png(bytes, channels, Some((columns, rows))).map(|(_, pixels)| pixels)
}

/// The greyscale PNG `bytes` hold, as its columns and rows and its raw pixel bytes: refused unless
/// it is of `channels`, and of `size` where one is asked for.
fn read_png(
    bytes: &[u8],
    channels: Channels,
    size: Option<(u32, u32)>,
) -> Result<((u32, u32), Vec<u8>), ContentProblem> {
    let unreadable = |found: String| ContentProblem::new("unreadable image", found, "a PNG");
    let mut reader = png::Decoder::new(Cursor::new(bytes))
        .read_info()
        .map_err(|error| unreadable(error.to_string()))?;
    let info = reader.info();
    let found = (info.width, info.height);
    if let Some((columns, rows)) = size.filter(|&size| size != found) {
        return Err(ContentProblem::new(
            "wrong size",
            format!("{} × {} px", found.0, found.1),
            format!("{columns} × {rows} px over this ground"),
        ));
    }
    let (depth, expected) = png_depth(channels);
    if (info.color_type, info.bit_depth) != (png::ColorType::Grayscale, depth) {
        return Err(ContentProblem::new(
            "wrong format",
            format!("{:?} {:?}", info.color_type, info.bit_depth),
            expected,
        ));
    }
    let buffer = reader
        .output_buffer_size()
        .ok_or_else(|| unreadable("an image too large to decode".to_owned()))?;
    let mut pixels = vec![0; buffer];
    let frame = reader
        .next_frame(&mut pixels)
        .map_err(|error| unreadable(error.to_string()))?;
    pixels.truncate(frame.buffer_size());
    pixels.shrink_to_fit();
    Ok((found, pixels))
}

/// Raw pixel `bytes` of `channels` as a `columns` × `rows` greyscale PNG.
fn encode_png(bytes: &[u8], columns: u32, rows: u32, channels: Channels) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, columns, rows);
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(png_depth(channels).0);
    let mut writer = encoder
        .write_header()
        .expect("a png header writes to memory");
    writer
        .write_image_data(bytes)
        .expect("png pixels write to memory");
    writer.finish().expect("a png finishes in memory");
    out
}

/// The PNG bit depth `channels` are kept at, and how a reader would name it.
fn png_depth(channels: Channels) -> (png::BitDepth, &'static str) {
    match channels {
        Channels::Grey16 => (png::BitDepth::Sixteen, "16-bit greyscale"),
        Channels::Grey8 => (png::BitDepth::Eight, "8-bit greyscale"),
    }
}
