//! The blades an area's covers grow, round where the scenery's sight looks from: every cell of the
//! ground within as far as the view still shows a piece's blades, as much of that as the player's
//! grass setting keeps, grown on the GPU from its paint and heights once they have loaded, the
//! nearest first, within the frame's upload budget, and let go once the sight leaves it behind.

use std::sync::Arc;

use bevy::platform::collections::HashSet;
use bevy::prelude::*;

use super::plan::{KEEP_SHARE, Seen};
use super::{Streaming, distance_to, ground_loaded};
use crate::core::math::Rect;
use crate::core::render::video::{GrassQuality, VideoPreferences};
use crate::core::render::world::gpu::grass::{
    GRASS_CELL_PIXELS, GRASS_HEIGHT_SIDE, GrassArena, GrassCell, GrassChange, GrassChanges,
    GrassGround, GrassPiece,
};
use crate::core::render::world::gpu::upload;
use crate::core::units::Meters;

/// What the scenery grows of its covers' blades: the pieces that grow them, what the GPU holds of
/// them, the cells grown for ground since sculpted or painted anew, and how thickly they were last
/// told to stand.
pub(super) struct Grass {
    pieces: Arc<[GrassPiece]>,
    arena: GrassArena,
    stale: HashSet<(u32, GrassCell)>,
    density: Option<f32>,
}

impl Grass {
    /// The grass of `pieces`, growing on the terrain `terrain`.
    pub(super) fn new(world: &mut World, terrain: Entity, pieces: Arc<[GrassPiece]>) -> Grass {
        if !pieces.is_empty()
            && let Some(mut changes) = world.get_resource_mut::<GrassChanges>()
        {
            changes
                .0
                .push((terrain, GrassChange::Pieces(pieces.clone())));
        }
        Grass {
            pieces,
            arena: GrassArena::default(),
            stale: HashSet::default(),
            density: None,
        }
    }

    /// Grows anew the blades on the stretches of ground in `changed`, sculpted or painted anew.
    pub(super) fn ground_changed(&mut self, changed: &[Rect<Meters>]) {
        self.stale.extend(self.arena.grown().filter(|(_, cell)| {
            let bounds = cell.bounds();
            changed.iter().any(|stretch| stretch.intersects(&bounds))
        }));
    }

    /// How many blades the GPU holds.
    pub(super) fn blades(&self) -> u64 {
        self.arena.blades()
    }
}

/// Grows the blades `seen` takes in and lets go of those it left behind: how many of its cells
/// have yet to grow.
pub(super) fn grow(world: &mut World, streaming: &mut Streaming, seen: &Seen) -> u32 {
    if streaming.grass.pieces.is_empty() {
        return 0;
    }
    let quality = world
        .get_resource::<VideoPreferences>()
        .map_or(GrassQuality::default(), |video| video.grass);
    let terrain = streaming.terrain;
    let mut changes = Vec::new();
    if streaming.grass.density != Some(quality.density()) {
        streaming.grass.density = Some(quality.density());
        changes.push(GrassChange::Thinned(quality.density()));
    }
    let Wanted { wanted, kept } = wanted_cells(streaming, seen, quality);
    let grass = &mut streaming.grass;
    let gone: Vec<(u32, GrassCell)> = grass
        .arena
        .grown()
        .filter(|grown| !kept.contains(grown))
        .collect();
    for (piece, cell) in gone {
        grass.stale.remove(&(piece, cell));
        changes.extend(grass.arena.cut(piece, cell));
    }
    let mut missing = 0;
    for (piece, cell) in wanted {
        let grass = &streaming.grass;
        if grass.arena.holds(piece, cell) && !grass.stale.contains(&(piece, cell)) {
            continue;
        }
        if !ground_loaded(streaming, cell.bounds()) || !upload::spend(world, GROUND_BYTES) {
            missing += 1;
            continue;
        }
        let ground = Arc::new(GrassGround::of(cell, &streaming.heights, &streaming.paint));
        let grass = &mut streaming.grass;
        grass.stale.remove(&(piece, cell));
        changes.extend(grass.arena.grow(&grass.pieces, piece, ground));
    }
    if let Some(mut held) = world.get_resource_mut::<GrassChanges>() {
        held.0
            .extend(changes.into_iter().map(|change| (terrain, change)));
    }
    missing
}

/// Lets the GPU go of all the grass of the terrain `terrain`.
pub(super) fn drop_grass(world: &mut World, terrain: Entity) {
    if let Some(mut changes) = world.get_resource_mut::<GrassChanges>() {
        changes.0.push((terrain, GrassChange::Dropped));
    }
}

/// The cells of every piece a sight takes in, nearest first, and those it keeps standing where they
/// are grown.
struct Wanted {
    wanted: Vec<(u32, GrassCell)>,
    kept: HashSet<(u32, GrassCell)>,
}

/// The cells of every piece `seen` takes in at `quality`: within the share of as far as the view
/// shows a piece's blades that `quality` keeps, from the nearest its eyes may come to the cell;
/// and kept within [`KEEP_SHARE`] times that.
fn wanted_cells(streaming: &Streaming, seen: &Seen, quality: GrassQuality) -> Wanted {
    let (columns, rows) = streaming.paint.pixels();
    let (cells_across, cells_down) = (
        columns.div_ceil(GRASS_CELL_PIXELS),
        rows.div_ceil(GRASS_CELL_PIXELS),
    );
    let lens = seen.lens();
    let ranges: Vec<Meters> = streaming
        .grass
        .pieces
        .iter()
        .map(|piece| Meters(lens.shown_within(piece.extent()).0 * quality.range()))
        .collect();
    let farthest = ranges.iter().map(|range| range.0).fold(0.0, f32::max) * KEEP_SHARE;
    let mut wanted = Vec::new();
    let mut kept = HashSet::default();
    for row in 0..cells_down {
        for column in 0..cells_across {
            let cell = GrassCell { column, row };
            let bounds = cell.bounds();
            if distance_to(bounds, seen.at()) > seen.reach.0.min(farthest) {
                continue;
            }
            let ground = streaming.heights.height_at(bounds.center());
            let from_eye = seen.over(bounds, ground, ground);
            for (piece, range) in ranges.iter().enumerate() {
                let piece = piece as u32;
                if from_eye <= range.0 * KEEP_SHARE {
                    kept.insert((piece, cell));
                }
                if from_eye <= range.0 {
                    wanted.push((from_eye, (piece, cell)));
                }
            }
        }
    }
    wanted.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    Wanted {
        wanted: wanted.into_iter().map(|(_, at)| at).collect(),
        kept,
    }
}

/// The bytes of the ground a cell grows on, as they go to the GPU: its paint, its heights and
/// where each of its rows starts.
const GROUND_BYTES: usize = (GRASS_CELL_PIXELS * GRASS_CELL_PIXELS) as usize
    + (GRASS_HEIGHT_SIDE * GRASS_HEIGHT_SIDE * 4) as usize
    + (GRASS_CELL_PIXELS * 4) as usize;
