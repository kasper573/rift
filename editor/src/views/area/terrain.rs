use std::collections::{HashMap, HashSet};

use engine::core::content::Content;
use engine::core::math::Rng;
use engine::core::units::{Cell, CellPos};
use engine::systems::area::grid::Flip;
use engine::systems::area::tileset::TilesetDef;

use super::grid::GridDraft;
use crate::views::tileset::terrain::{MixCompleteness, TileCorner};

pub struct TerrainBrush<'a> {
    pub def: &'a TilesetDef,
    pub tileset: &'a str,
    pub set: &'a str,
    pub terrain: usize,
    pub size: u32,
}

pub fn paint(
    draft: &mut GridDraft,
    layer: usize,
    brush: &TerrainBrush,
    center: CellPos,
    rng: &mut Rng,
    painted: &mut HashSet<CellPos>,
) -> usize {
    let TerrainBrush {
        def,
        tileset,
        set,
        terrain,
        size,
    } = *brush;
    let Some(terrains) = def.terrains.get(set) else {
        return 0;
    };
    let Some(name) = terrains.get(terrain) else {
        return 0;
    };
    let base = &terrains[0];
    let plain = || std::array::from_fn(|_| base.clone());
    let half = (size.max(1) as i32 - 1) / 2;
    let extra = i32::from(size.max(1).is_multiple_of(2));
    let mut vertices: Vec<(i32, i32)> = Vec::new();
    for dy in -half - extra..=half + 1 {
        for dx in -half - extra..=half + 1 {
            vertices.push((center.x + dx, center.y + dy));
        }
    }
    let mut touched: HashSet<CellPos> = HashSet::new();
    let corners_of = |cell: CellPos| -> Corners {
        let Some(row) = draft
            .get(layer, cell)
            .and_then(|index| draft.entry(index).cloned())
        else {
            return Corners::Empty;
        };
        (row.tileset == tileset)
            .then(|| def.data(row.tile))
            .flatten()
            .and_then(|data| data.corners.get(set))
            .cloned()
            .map_or(Corners::Foreign, Corners::Known)
    };
    let mut foreign: HashSet<CellPos> = HashSet::new();
    let mut wanted: HashMap<CellPos, [String; 4]> = HashMap::new();
    for (vx, vy) in vertices {
        let around = [
            (CellPos::new(vx - 1, vy - 1), TileCorner::SouthEast),
            (CellPos::new(vx, vy - 1), TileCorner::SouthWest),
            (CellPos::new(vx - 1, vy), TileCorner::NorthEast),
            (CellPos::new(vx, vy), TileCorner::NorthWest),
        ];
        for (cell, corner) in around {
            if cell.index(draft.size).is_none() {
                continue;
            }
            let entry = wanted
                .entry(cell)
                .or_insert_with(|| match corners_of(cell) {
                    Corners::Known(corners) => corners,
                    Corners::Empty => plain(),
                    Corners::Foreign => {
                        foreign.insert(cell);
                        plain()
                    }
                });
            entry[corner.index()] = name.clone();
            touched.insert(cell);
        }
    }
    let mut kept = 0;
    for cell in touched {
        let corners = &wanted[&cell];
        if corners.iter().all(|c| c == base) {
            if foreign.contains(&cell) {
                continue;
            }
            if draft.set(layer, cell, None) {
                painted.insert(cell);
            }
            continue;
        }
        let Some(tile) = rng.choose(&def.tiles_with_corners(set, corners)).copied() else {
            kept += 1;
            continue;
        };
        let index = draft.intern(tileset, tile, Flip::None);
        if draft.set(layer, cell, Some(index)) {
            painted.insert(cell);
        }
    }
    kept
}

pub fn repaint(
    draft: &mut GridDraft,
    layer: usize,
    def: &TilesetDef,
    tileset: &str,
    set: &str,
    rng: &mut Rng,
) -> (usize, usize) {
    let cells: Vec<CellPos> = draft.cells().collect();
    let (mut changed, mut kept) = (0, 0);
    for cell in cells {
        let Some(row) = draft
            .get(layer, cell)
            .and_then(|index| draft.entry(index).cloned())
        else {
            continue;
        };
        if row.tileset != tileset {
            continue;
        }
        let Some(corners) = def
            .data(row.tile)
            .and_then(|data| data.corners.get(set))
            .cloned()
        else {
            continue;
        };
        let Some(pick) = rng.choose(&def.tiles_with_corners(set, &corners)).copied() else {
            kept += 1;
            continue;
        };
        let index = draft.intern(tileset, pick, Flip::None);
        if draft.set(layer, cell, Some(index)) {
            changed += 1;
        }
    }
    (changed, kept)
}

pub struct TerrainSet {
    pub tileset: String,
    pub set: String,
    pub terrains: Vec<String>,
}

impl TerrainSet {
    /// How many of the set's mixes have a tile.
    pub fn coverage(&self, content: &Content) -> MixCompleteness {
        content
            .by_path::<TilesetDef>(&self.tileset)
            .map(|id| MixCompleteness::of(id.get(content), &self.set))
            .unwrap_or_default()
    }
}

pub fn sets_for(content: &Content, tile_px: u32) -> Vec<TerrainSet> {
    content
        .table::<TilesetDef>()
        .iter()
        .filter(|(_, def)| def.tile == tile_px)
        .flat_map(|(id, def)| {
            let tileset = id.path(content).to_owned();
            def.terrains.iter().map(move |(set, terrains)| TerrainSet {
                tileset: tileset.clone(),
                set: set.clone(),
                terrains: terrains.clone(),
            })
        })
        .collect()
}

/// A cell's corners in the brush's set, as its tile says. A foreign tile, of another tileset or
/// knowing no corners in the set, isn't the set's base: a stroke only leaving base around it keeps it.
enum Corners {
    Empty,
    Known([String; 4]),
    Foreign,
}
