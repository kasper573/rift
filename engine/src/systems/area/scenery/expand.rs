//! What an area's scenery stands within a chunk of its ground, worked out from the chunk and the
//! area's content alone: a worker works any chunk out on its own, and the same seeds place the
//! same props and copies on every machine, whichever chunks they are worked out by.

use crate::core::ground::{Heightfield, PaintMap, SAMPLES_PER_METER};
use std::ops::Range;

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
        let (across, down) = counts(ground);
        ChunkSpan {
            columns: 0..across,
            rows: 0..down,
        }
        .into_iter()
    }

    /// Every chunk of `ground` that `rect` reaches into, those on its far edges among them.
    pub fn within(ground: Rect<Meters>, rect: Rect<Meters>) -> ChunkSpan {
        let side = Self::SIDE.0;
        let (across, down) = counts(ground);
        let first = |at: f32, origin: f32, count: u32| {
            (((at - origin) / side).floor() as i64).clamp(0, i64::from(count)) as u32
        };
        let past = |at: f32, origin: f32, count: u32| {
            ((((at - origin) / side).floor() as i64) + 1).clamp(0, i64::from(count)) as u32
        };
        ChunkSpan {
            columns: first(rect.min_x(), ground.origin.x, across)
                ..past(rect.max_x(), ground.origin.x, across),
            rows: first(rect.min_y(), ground.origin.y, down)
                ..past(rect.max_y(), ground.origin.y, down),
        }
    }

    /// The chunk of `ground` that `at` lies in, if any: a point on a chunk's far edges lies in
    /// the chunk beyond, so every point lies in one chunk alone.
    pub fn at(ground: Rect<Meters>, at: Pos<Meters>) -> Option<SceneryChunk> {
        let side = Self::SIDE.0;
        let (across, down) = counts(ground);
        let column = ((at.x - ground.origin.x) / side).floor();
        let row = ((at.y - ground.origin.y) / side).floor();
        let within = column >= 0.0 && column < across as f32 && row >= 0.0 && row < down as f32;
        within.then_some(SceneryChunk {
            column: column as u32,
            row: row as u32,
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
        Self::at(ground, at) == Some(self)
    }
}

/// A rectangle of chunks: those in `columns` across each of `rows` down.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkSpan {
    pub columns: Range<u32>,
    pub rows: Range<u32>,
}

impl ChunkSpan {
    /// Whether the span takes in `chunk`.
    pub fn holds(&self, chunk: SceneryChunk) -> bool {
        self.columns.contains(&chunk.column) && self.rows.contains(&chunk.row)
    }

    /// The chunks of the span, row by row.
    pub fn iter(&self) -> impl Iterator<Item = SceneryChunk> + use<> {
        let columns = self.columns.clone();
        self.rows.clone().flat_map(move |row| {
            columns
                .clone()
                .map(move |column| SceneryChunk { column, row })
        })
    }
}

impl IntoIterator for ChunkSpan {
    type Item = SceneryChunk;
    type IntoIter = Box<dyn Iterator<Item = SceneryChunk>>;

    fn into_iter(self) -> Self::IntoIter {
        Box::new(self.iter())
    }
}

/// Which of an area's prop sources reach each chunk of its ground, filed once from where each
/// stands or strews, so that working a chunk out looks at the sources that may stand anything in
/// it alone, rather than at every source of the area.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourcesByChunk {
    across: u32,
    reaching: Vec<Vec<u32>>,
}

impl SourcesByChunk {
    pub fn of<'a>(
        modules: impl IntoIterator<Item = &'a AreaModule>,
        ground: Rect<Meters>,
    ) -> SourcesByChunk {
        let (across, down) = counts(ground);
        let mut filed = SourcesByChunk {
            across,
            reaching: vec![Vec::new(); (across * down) as usize],
        };
        for (source, module) in modules.into_iter().enumerate() {
            let source = source as u32;
            match module {
                AreaModule::Prop(prop) => {
                    if let Some(chunk) = SceneryChunk::at(ground, prop.at) {
                        filed.file(chunk, source);
                    }
                }
                AreaModule::Scatter(scatter) => {
                    for chunk in SceneryChunk::within(ground, scatter.region) {
                        filed.file(chunk, source);
                    }
                }
                _ => {}
            }
        }
        filed
    }

    /// [`props_in`] of the sources filed as reaching `chunk`, each prop with the place among
    /// `modules` of the module standing it.
    pub fn sourced_props_in(
        &self,
        modules: &[AreaModule],
        ground: Rect<Meters>,
        chunk: SceneryChunk,
    ) -> Vec<(u32, PropPlacement)> {
        let sources = self
            .reaching(chunk)
            .iter()
            .map(|&source| (source, &modules[source as usize]));
        props_from(sources, ground, chunk)
    }

    /// The props standing in every chunk of `span`, each with the place among `modules` of the
    /// module standing it and the chunk it stands in: the same props as [`props_in`] of each
    /// chunk, worked out with each source expanded once over the whole span.
    pub fn sourced_props_within(
        &self,
        modules: &[AreaModule],
        ground: Rect<Meters>,
        span: &ChunkSpan,
    ) -> Vec<(u32, SceneryChunk, PropPlacement)> {
        let mut sources: Vec<u32> = span
            .iter()
            .flat_map(|chunk| self.reaching(chunk).iter().copied())
            .collect();
        sources.sort_unstable();
        sources.dedup();
        let mut props = Vec::new();
        for source in sources {
            let chunk_within = |placed: &PropPlacement| {
                SceneryChunk::at(ground, placed.at).filter(|chunk| span.holds(*chunk))
            };
            match &modules[source as usize] {
                AreaModule::Prop(prop) => {
                    if let Some(chunk) = chunk_within(prop) {
                        props.push((source, chunk, prop.clone()));
                    }
                }
                AreaModule::Scatter(scatter) => {
                    props.extend(scatter.expand().filter_map(|placed| {
                        chunk_within(&placed).map(|chunk| (source, chunk, placed))
                    }))
                }
                _ => {}
            }
        }
        props
    }

    fn reaching(&self, chunk: SceneryChunk) -> &[u32] {
        self.reaching
            .get((chunk.row * self.across + chunk.column) as usize)
            .map_or(&[], Vec::as_slice)
    }

    fn file(&mut self, chunk: SceneryChunk, source: u32) {
        self.reaching[(chunk.row * self.across + chunk.column) as usize].push(source);
    }
}

/// The props an area of `modules` stands in `chunk` of its `ground`, placed and scattered: the
/// same, bit for bit and in the same order, as those of the area's props that stand there.
pub fn props_in<'a>(
    modules: impl IntoIterator<Item = &'a AreaModule>,
    ground: Rect<Meters>,
    chunk: SceneryChunk,
) -> Vec<PropPlacement> {
    sourced_props_in(modules, ground, chunk)
        .into_iter()
        .map(|(_, placed)| placed)
        .collect()
}

/// [`props_in`], each prop with the place among `modules` of the module standing it.
pub fn sourced_props_in<'a>(
    modules: impl IntoIterator<Item = &'a AreaModule>,
    ground: Rect<Meters>,
    chunk: SceneryChunk,
) -> Vec<(u32, PropPlacement)> {
    let sources = modules
        .into_iter()
        .enumerate()
        .map(|(source, module)| (source as u32, module));
    props_from(sources, ground, chunk)
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
        .filter(|placed| chunk.holds(ground, placed.at))
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

/// The props the `sources` stand in `chunk` of `ground`, each with its source's place.
fn props_from<'a>(
    sources: impl Iterator<Item = (u32, &'a AreaModule)>,
    ground: Rect<Meters>,
    chunk: SceneryChunk,
) -> Vec<(u32, PropPlacement)> {
    let mut props = Vec::new();
    for (source, module) in sources {
        match module {
            AreaModule::Prop(prop) if chunk.holds(ground, prop.at) => {
                props.push((source, prop.clone()));
            }
            AreaModule::Scatter(scatter) => props.extend(
                scattered_in(scatter, ground, chunk)
                    .into_iter()
                    .map(|placed| (source, placed)),
            ),
            _ => {}
        }
    }
    props
}

/// How many chunks `ground` has across and down.
fn counts(ground: Rect<Meters>) -> (u32, u32) {
    let side = SceneryChunk::SIDE.0;
    (
        (ground.size.width / side).ceil() as u32,
        (ground.size.height / side).ceil() as u32,
    )
}
