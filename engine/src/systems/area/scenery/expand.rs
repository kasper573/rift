//! What an area's scenery stands within a chunk of its ground, worked out from the chunk and the
//! area's content alone: a worker works any chunk out on its own, and the same seeds place the
//! same props and copies on every machine, whichever chunks they are worked out by.

use crate::core::ground::{Heightfield, PaintMap, SAMPLES_PER_METER};
use crate::core::math::{Pos, Rect, Size};
use crate::core::render::world::cover::{self, Place, Spread};
use crate::core::units::{GROUND_ORIGIN, Meters};
use crate::systems::area::prop::Scatter;
use crate::systems::area::{AreaModule, PropPlacement};

use super::PLACEMENT_CELL;

/// A chunk of an area's ground, by its place among the chunks across and down from the ground's
/// corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SceneryChunk {
    pub column: u32,
    pub row: u32,
}

impl SceneryChunk {
    pub const SIDE: Meters = PLACEMENT_CELL;

    /// Every chunk of `ground`, row by row.
    pub fn all(ground: Rect<Meters>) -> impl Iterator<Item = SceneryChunk> {
        let across = (ground.size.width / Self::SIDE.0).ceil() as u32;
        let down = (ground.size.height / Self::SIDE.0).ceil() as u32;
        (0..down).flat_map(move |row| (0..across).map(move |column| SceneryChunk { column, row }))
    }

    /// Every chunk of `ground` that `rect` reaches into, row by row.
    pub fn within(ground: Rect<Meters>, rect: Rect<Meters>) -> impl Iterator<Item = SceneryChunk> {
        let side = Self::SIDE.0;
        let across = (ground.size.width / side).ceil() as i64;
        let down = (ground.size.height / side).ceil() as i64;
        let first = |at: f32, origin: f32, count: i64| {
            (((at - origin) / side).floor() as i64).clamp(0, count) as u32
        };
        let past = |at: f32, origin: f32, count: i64| {
            ((((at - origin) / side).floor() as i64) + 1).clamp(0, count) as u32
        };
        let columns = first(rect.min_x(), ground.origin.x, across)
            ..past(rect.max_x(), ground.origin.x, across);
        let rows =
            first(rect.min_y(), ground.origin.y, down)..past(rect.max_y(), ground.origin.y, down);
        rows.flat_map(move |row| {
            columns
                .clone()
                .map(move |column| SceneryChunk { column, row })
        })
    }

    /// The ground of `ground` the chunk covers. A point on its far edges lies in the chunk beyond,
    /// so every point lies in one chunk alone.
    pub fn bounds(self, ground: Rect<Meters>) -> Rect<Meters> {
        let side = Self::SIDE.0;
        Rect::new(
            Pos::new(
                ground.origin.x + self.column as f32 * side,
                ground.origin.y + self.row as f32 * side,
            ),
            Size::new(side, side),
        )
    }

    /// Whether `at` lies in the chunk of `ground`.
    pub fn holds(self, ground: Rect<Meters>, at: Pos<Meters>) -> bool {
        holds(self.bounds(ground), at)
    }
}

/// The props an area of `modules` stands in `chunk` of its `ground`, placed and scattered: the
/// same, bit for bit and in the same order, as those of the area's props that stand there.
pub fn props_in<'a>(
    modules: impl IntoIterator<Item = &'a AreaModule>,
    ground: Rect<Meters>,
    chunk: SceneryChunk,
) -> Vec<PropPlacement> {
    let mut props = Vec::new();
    for module in modules {
        match module {
            AreaModule::Prop(prop) if chunk.holds(ground, prop.at) => props.push(prop.clone()),
            AreaModule::Scatter(scatter) => props.extend(scattered_in(scatter, ground, chunk)),
            _ => {}
        }
    }
    props
}

/// The props `scatter` strews that stand in `chunk` of `ground`: the same, bit for bit and in the
/// same order, as those of [`Scatter::expand`] that stand there.
pub fn scattered_in(
    scatter: &Scatter,
    ground: Rect<Meters>,
    chunk: SceneryChunk,
) -> Vec<PropPlacement> {
    let bounds = chunk.bounds(ground);
    let region = scatter.region;
    // A prop lands at most on the region's far edges, which the chunk past them may hold.
    let apart = region.max_x() < bounds.min_x()
        || region.max_y() < bounds.min_y()
        || region.min_x() >= bounds.max_x()
        || region.min_y() >= bounds.max_y();
    if apart {
        return Vec::new();
    }
    scatter
        .expand()
        .filter(|placed| holds(bounds, placed.at))
        .collect()
}

/// A chunk of the paint, by its place among the chunks across and down.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PaintChunk {
    pub column: u32,
    pub row: u32,
}

impl PaintChunk {
    /// How far a chunk reaches along each side.
    pub const SIDE: Meters = Meters(cover::CHUNK_CELLS as f32 / SAMPLES_PER_METER as f32);

    /// The cell of paint the chunk begins at.
    pub fn first_cell(self) -> (u32, u32) {
        (
            self.column * cover::CHUNK_CELLS,
            self.row * cover::CHUNK_CELLS,
        )
    }

    /// The ground the chunk's cells cover, from the corner of its first.
    pub fn bounds(self) -> Rect<Meters> {
        let cell = 1.0 / SAMPLES_PER_METER as f32;
        let side = cover::CHUNK_CELLS as f32 * cell;
        let (column, row) = self.first_cell();
        Rect::new(
            Pos::new(
                GROUND_ORIGIN.x + column as f32 * cell,
                GROUND_ORIGIN.y + row as f32 * cell,
            ),
            Size::new(side, side),
        )
    }
}

/// A cover as it strews: the slot of the terrain that grows it among the area's terrains, and the
/// cover's seed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StrewnCover {
    pub slot: usize,
    pub seed: u64,
}

/// Where the piece of `cover` spread by `spread` strews its copies in `chunk` of the ground of
/// `heights` painted by `paint`: drawn from the chunk's own seed over the cells of the chunk alone,
/// so a chunk strews alike whatever lies around it.
pub fn strewn_in(
    heights: &Heightfield,
    paint: &PaintMap,
    cover: StrewnCover,
    spread: &Spread,
    chunk: PaintChunk,
) -> Vec<Place> {
    let (columns, rows) = paint.pixels();
    let first = chunk.first_cell();
    if first.0 >= columns || first.1 >= rows {
        return Vec::new();
    }
    let share = painted_by(paint, cover.slot);
    let patch = cover::Patch {
        heights,
        first,
        columns: cover::CHUNK_CELLS.min(columns - first.0),
        rows: cover::CHUNK_CELLS.min(rows - first.1),
        share: &share,
    };
    cover::strew_places(&patch, spread, chunk_seed(cover, spread, chunk))
}

/// The share of each cell of paint that `slot` paints: all of it or none.
pub(super) fn painted_by(paint: &PaintMap, slot: usize) -> impl Fn(u32, u32) -> f32 + '_ {
    move |column, row| {
        if usize::from(paint.index(column, row)) == slot {
            1.0
        } else {
            0.0
        }
    }
}

/// A seed of its own for every chunk of every piece, mixed from the cover's seed and where the
/// chunk lies, so changing the cover's seed rescatters everything and nothing else repeats.
fn chunk_seed(cover: StrewnCover, spread: &Spread, chunk: PaintChunk) -> u64 {
    mixed_seed(
        cover.seed,
        [
            cover.slot as u64,
            u64::from(spread.piece),
            u64::from(chunk.row),
            u64::from(chunk.column),
        ],
    )
}

pub(super) fn mixed_seed(seed: u64, parts: [u64; 4]) -> u64 {
    let mut mixed = seed ^ 0x9E37_79B9_7F4A_7C15;
    for part in parts {
        mixed ^= part.wrapping_add(0x9E37_79B9_7F4A_7C15);
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        mixed ^= mixed >> 31;
    }
    mixed
}

/// Whether `at` lies within `bounds`, its far edges left to the chunks beyond, so a point lies in
/// one chunk alone.
fn holds(bounds: Rect<Meters>, at: Pos<Meters>) -> bool {
    at.x >= bounds.min().x
        && at.x < bounds.max().x
        && at.y >= bounds.min().y
        && at.y < bounds.max().y
}
