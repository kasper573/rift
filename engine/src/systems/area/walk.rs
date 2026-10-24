use super::Scenery;
use super::grid::AreaGrids;
use super::tileset::{TilesetDef, Walk};
use crate::core::audio::playback::SfxId;
use crate::core::content::Content;
use crate::core::math::{Pos, Rect, Size};
use crate::core::units::{Cell, CellPos, GridDims, GridSize, Meters, TilePos, TileRect};

const OBSCURING_CUTOFF: f32 = 0.4;

pub struct WalkGrids {
    pub walk: Vec<bool>,
    pub wild: Vec<bool>,
    pub airspace: Vec<bool>,
    pub wild_airspace: Vec<bool>,
    pub safe_cells: Vec<bool>,
    pub footsteps: Vec<Option<SfxId>>,
}

pub fn grids(
    content: &Content,
    grids: &AreaGrids,
    scenery: &[Scenery],
    safe_zones: &[Rect<Meters>],
) -> WalkGrids {
    let size = grids.size;
    let votes = votes(content, grids, scenery);
    let safe_cells: Vec<bool> = size
        .cells()
        .map(|cell| safe_zones.iter().any(|zone| zone.contains(cell.center())))
        .collect();
    let walk: Vec<bool> = votes.iter().map(|votes| votes.walkable()).collect();
    let wild = walk
        .iter()
        .zip(&safe_cells)
        .map(|(&walkable, &safe)| walkable && !safe)
        .collect();
    let airspace = vec![true; walk.len()];
    let wild_airspace = safe_cells.iter().map(|safe| !safe).collect();
    WalkGrids {
        walk,
        wild,
        airspace,
        wild_airspace,
        safe_cells,
        footsteps: footsteps(content, grids),
    }
}

pub fn tile_square(scenery: &Scenery, column: usize, row: usize) -> Rect<Meters> {
    let cell = Rect::new(Pos::new(column as f32, row as f32), Size::splat(1.0));
    scenery.flip.place(cell, scenery.at)
}

pub fn in_map(size: GridSize, at: Pos<Meters>) -> bool {
    at.cell().index(size).is_some()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct CellVotes {
    grid: Option<bool>,
    blocks: bool,
    walks: bool,
}

impl CellVotes {
    fn vote(&mut self, walk: Option<Walk>) {
        match walk {
            Some(Walk::Blocks) => self.blocks = true,
            Some(Walk::Walkable) => self.walks = true,
            None => {}
        }
    }

    fn walkable(self) -> bool {
        match self.grid {
            Some(value) => value,
            None => !self.blocks && self.walks,
        }
    }
}

fn votes(content: &Content, grids: &AreaGrids, scenery: &[Scenery]) -> Vec<CellVotes> {
    let size = grids.size;
    let mut votes: Vec<CellVotes> = grids
        .walk
        .iter()
        .map(|&grid| CellVotes {
            grid,
            ..CellVotes::default()
        })
        .collect();
    for layer in &grids.ground {
        for (index, cell) in layer.cells.iter().enumerate() {
            if let Some(entry) = grids.entry(*cell) {
                votes[index].vote(entry.tileset.get(content).walk(entry.tile));
            }
        }
    }
    for scenery in scenery {
        let tileset = scenery.tileset.get(content);
        for (tile, rect) in scenery_tiles(scenery) {
            let walk = tileset.walk(tile);
            if walk.is_none() {
                continue;
            }
            for cell in rect.tiles() {
                if cell_overlap(&rect, cell) >= OBSCURING_CUTOFF
                    && let Some(index) = cell.index(size)
                {
                    votes[index].vote(walk);
                }
            }
        }
        for cell in scenery_blocks(scenery) {
            if let Some(index) = cell.index(size) {
                votes[index].blocks = true;
            }
        }
    }
    votes
}

fn scenery_tiles(scenery: &Scenery) -> impl Iterator<Item = (u32, Rect<Meters>)> + '_ {
    scenery
        .tiles
        .iter()
        .enumerate()
        .flat_map(move |(row, tiles)| {
            tiles.iter().enumerate().filter_map(move |(column, tile)| {
                let tile = tile.as_ref()?;
                Some((tile.tile, tile_square(scenery, column, row)))
            })
        })
}

fn scenery_blocks(scenery: &Scenery) -> impl Iterator<Item = CellPos> + '_ {
    scenery
        .blocks
        .iter()
        .map(|&block| scenery.cell_under(block))
}

fn cell_overlap(rect: &Rect<Meters>, cell: CellPos) -> f32 {
    rect.intersection(&cell.bounds())
        .map_or(0.0, |overlap| overlap.area())
}

fn footsteps(content: &Content, grids: &AreaGrids) -> Vec<Option<SfxId>> {
    let mut footsteps = vec![None; (grids.size.width * grids.size.height) as usize];
    for layer in &grids.ground {
        for (index, cell) in layer.cells.iter().enumerate() {
            let Some(entry) = grids.entry(*cell) else {
                continue;
            };
            let tileset: &TilesetDef = entry.tileset.get(content);
            if let Some(surface) = tileset.surface(entry.tile) {
                footsteps[index] = Some(surface.get(content).footstep);
            }
        }
    }
    footsteps
}
