//! Builds an [`Area`] from its Tiled `.tmx` map: render layers, the tile palette, depth groups,
//! the nav grid, portals, and per-tile sound.

use std::collections::HashSet;

use tiled::{LayerType, PropertyValue};

use super::{Area, AreaDef, Flip, Group, Portal, RenderLayer, TileDef, TileRef, cell_overlap};
use crate::content::actors::SfxId;
use crate::core::assets;
use crate::core::math::{Offset, Pos, Rect, Size, WorldPx};
use crate::core::nav;
use crate::core::table::Id;
use crate::core::tiling::{
    self, Cell, CellPos, GridDims, GridSize, PixelsPerTile, TileRect, TileSize, Tiles,
};
use crate::core::time::Millis;

const OBSCURING_CUTOFF: f32 = 0.4;

pub(super) fn build_area(id: Id<AreaDef>, name: &str, map_name: &str) -> Area {
    let map = load_map(map_name);
    let tiling = PixelsPerTile::new(Size::new(map.tile_width as f32, map.tile_height as f32));
    let size = Size::new(map.width as f32, map.height as f32);

    let mut tiles = TilePalette::default();
    let mut layers = Vec::new();
    let mut objects = Vec::new();
    let mut start = None;
    let mut portals = Vec::new();
    let mut obscuring_rects = Vec::new();

    for layer in map.layers() {
        match layer.layer_type() {
            LayerType::Tiles(tile_layer) => {
                let dims = GridSize::new(map.width, map.height);
                let mut cells = vec![TileRef::EMPTY; (dims.width * dims.height) as usize];
                for (i, cell) in dims.cells().enumerate() {
                    if let Some(tile) = tile_layer.get_tile(cell.x, cell.y) {
                        cells[i] = tiles.add(
                            tile.get_tileset(),
                            tile.id(),
                            Flip {
                                x: tile.flip_h,
                                y: tile.flip_v,
                            },
                        );
                    }
                }
                layers.push(RenderLayer {
                    dynamic: layer.name.eq_ignore_ascii_case("Dynamic"),
                    size: dims,
                    cells,
                });
            }
            LayerType::Objects(object_layer) => {
                for object in object_layer.objects() {
                    let pos = Pos::new(object.x, object.y);
                    if let Some(object_tile) = object.get_tile() {
                        let data = object.tile_data().expect("tile object has tile data");
                        let tileset = object_tile.get_tileset();
                        let cell = tiles.add(
                            tileset,
                            data.id(),
                            Flip {
                                x: data.flip_h,
                                y: data.flip_v,
                            },
                        );
                        objects.push((tiling.point(pos), cell));
                        let obscuring = object_tile.get_tile().is_some_and(|tile| {
                            tile.properties.get("Walkable") != Some(&PropertyValue::BoolValue(true))
                        });
                        if obscuring {
                            let object_size = shape_size(&object.shape);
                            obscuring_rects.push(tiling.rect(Rect::new(
                                pos - Offset::new(0.0, object_size.height),
                                object_size,
                            )));
                        }
                        continue;
                    }
                    match object.user_type.as_str() {
                        "start" => start = Some(tiling.tile_center(pos)),
                        "warp" => {
                            let goto = match object.properties.get("goto") {
                                Some(PropertyValue::StringValue(goto)) => goto,
                                _ => panic!(
                                    "map '{name}': warp {} lacks a goto property",
                                    object.id()
                                ),
                            };
                            portals.push(portal(name, goto, pos, &object.shape, tiling));
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    let spawn = start.unwrap_or_else(|| panic!("map '{name}' must place a start object"));
    if !layers.iter().any(|layer| layer.dynamic) {
        panic!("map '{name}' must have a 'Dynamic' tile layer");
    }
    let grid = build_grid(size, &layers, &tiles, &obscuring_rects);
    let walkable_nodes = size
        .grid()
        .cells()
        .map(|c| c.center())
        .filter(|&p| grid.walkable(p))
        .collect();
    let (groups, grouped_cells) = compute_groups(&layers, &tiles);
    let tile_sfx = tile_sfx(size, &layers, &tiles);

    Area {
        id,
        name: name.to_owned(),
        size,
        grid,
        tile_sfx,
        spawn,
        portals,
        walkable_nodes,
        obscuring_rects,
        objects,
        groups,
        grouped_cells,
        layers,
        tiles: tiles.defs,
    }
}

fn load_map(name: &str) -> tiled::Map {
    tiled::Loader::with_reader(assets::tiled_reader)
        .load_tmx_map(format!("{}/{name}.tmx", assets::MAPS))
        .unwrap_or_else(|error| panic!("map '{name}': {error}"))
}

#[derive(Default)]
struct TilePalette {
    keys: Vec<(usize, u32)>,
    defs: Vec<TileDef>,
    walkable: Vec<Option<bool>>,
    group: Vec<Option<i64>>,
    sfx: Vec<Option<SfxId>>,
}

impl TilePalette {
    fn add(&mut self, tileset: &tiled::Tileset, id: u32, flip: Flip) -> TileRef {
        let identity = tileset as *const tiled::Tileset as usize;
        let key = (identity, id);
        let index = match self.keys.iter().position(|&k| k == key) {
            Some(index) => index,
            None => {
                self.keys.push(key);
                self.defs.push(tile_def(tileset, id));
                let properties = tileset
                    .get_tile(id)
                    .map(|tile| tile.properties.clone())
                    .unwrap_or_default();
                self.walkable.push(match properties.get("Walkable") {
                    Some(PropertyValue::BoolValue(walkable)) => Some(*walkable),
                    _ => None,
                });
                self.group.push(match properties.get("Group") {
                    Some(PropertyValue::IntValue(group)) => Some(*group as i64),
                    Some(PropertyValue::FloatValue(group)) => Some(*group as i64),
                    Some(PropertyValue::StringValue(group)) => group.parse().ok(),
                    _ => None,
                });
                self.sfx.push(match properties.get("sfx") {
                    Some(PropertyValue::StringValue(sfx)) => Some(SfxId(sfx.clone())),
                    _ => None,
                });
                self.defs.len() - 1
            }
        };
        TileRef::new(index, flip)
    }

    fn walkable_of(&self, cell: TileRef) -> Option<bool> {
        self.walkable[cell.index()?]
    }

    fn group_of(&self, cell: TileRef) -> Option<i64> {
        self.group[cell.index()?]
    }

    fn sfx_of(&self, cell: TileRef) -> Option<&SfxId> {
        self.sfx[cell.index()?].as_ref()
    }
}

fn tile_def(tileset: &tiled::Tileset, id: u32) -> TileDef {
    let image = tileset
        .image
        .as_ref()
        .unwrap_or_else(|| panic!("tileset {} must be atlas-based", tileset.name));
    let source = image
        .source
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_else(|| panic!("tileset {} image source", tileset.name));
    let sheet = assets::find(assets::TILESETS, source)
        .unwrap_or_else(|| panic!("unknown tileset image {source}"));

    let region = |id: u32| {
        let columns = tileset.columns.max(1);
        Rect::new(
            Pos::new(
                (tileset.margin + (id % columns) * (tileset.tile_width + tileset.spacing)) as f32,
                (tileset.margin + (id / columns) * (tileset.tile_height + tileset.spacing)) as f32,
            ),
            Size::new(tileset.tile_width as f32, tileset.tile_height as f32),
        )
    };

    let animation = tileset.get_tile(id).and_then(|tile| tile.animation.clone());
    let frames: Vec<(Rect<WorldPx>, Millis)> = match animation {
        Some(frames) if !frames.is_empty() => frames
            .iter()
            .map(|frame| (region(frame.tile_id), Millis(frame.duration as f32)))
            .collect(),
        _ => vec![(region(id), Millis(0.0))],
    };
    let total = Millis(frames.iter().map(|&(_, duration)| duration.0).sum());
    TileDef {
        sheet,
        frames,
        total,
    }
}

fn shape_size(shape: &tiled::ObjectShape) -> Size<WorldPx> {
    match shape {
        tiled::ObjectShape::Rect { width, height }
        | tiled::ObjectShape::Ellipse { width, height } => Size::new(*width, *height),
        _ => Size::splat(0.0),
    }
}

fn portal(
    name: &str,
    goto: &str,
    pos: Pos<WorldPx>,
    shape: &tiled::ObjectShape,
    tiling: PixelsPerTile,
) -> Portal {
    let malformed = || -> ! { panic!("map '{name}': goto '{goto}' must be '<area>, x, y'") };
    let mut parts = goto.split(',');
    let dest = parts.next().unwrap_or_else(|| malformed()).trim();
    let dest_area = Id::<AreaDef>::by_name(dest)
        .unwrap_or_else(|| panic!("map '{name}': unknown area '{dest}'"));
    let mut coord = || -> f32 {
        parts
            .next()
            .and_then(|part| part.trim().parse().ok())
            .unwrap_or_else(|| malformed())
    };
    Portal {
        rect: tiling.rect(Rect::new(pos, shape_size(shape))),
        dest_area,
        dest: Pos::new(coord(), coord()),
    }
}

fn compute_groups(layers: &[RenderLayer], tiles: &TilePalette) -> (Vec<Group>, HashSet<CellPos>) {
    let mut groups = Vec::new();
    let mut grouped_cells = HashSet::new();
    let Some(dynamic) = layers.iter().find(|layer| layer.dynamic) else {
        return (groups, grouped_cells);
    };
    let group_at = |c: CellPos| -> Option<i64> { tiles.group_of(dynamic.at(c)) };

    let mut visited: HashSet<CellPos> = HashSet::new();
    for start in dynamic.size.cells() {
        let Some(group_id) = group_at(start) else {
            continue;
        };
        if !visited.insert(start) {
            continue;
        }
        let mut stack = vec![start];
        let mut cells = Vec::new();
        let mut bottom = start.y;
        while let Some(c) = stack.pop() {
            cells.push((c, dynamic.at(c)));
            grouped_cells.insert(c);
            bottom = bottom.max(c.y);
            for step in tiling::NEIGHBORS_4 {
                let next = c.step(step);
                if group_at(next) == Some(group_id) && visited.insert(next) {
                    stack.push(next);
                }
            }
        }
        groups.push(Group {
            // Depth sorts against actor/object centers (see tiling.rs); anchoring at the
            // bottom row's near edge keeps a player on that row in front, never tying.
            bottom: Tiles(CellPos::new(0, bottom).bounds().min().y),
            tiles: cells,
        });
    }
    (groups, grouped_cells)
}

fn tile_sfx(size: Size<Tiles>, layers: &[RenderLayer], tiles: &TilePalette) -> Vec<Option<SfxId>> {
    let grid = size.grid();
    let mut sfx = vec![None; (grid.width * grid.height) as usize];
    for layer in layers {
        for (i, cell) in grid.cells().enumerate() {
            if let Some(id) = tiles.sfx_of(layer.at(cell)) {
                sfx[i] = Some(id.clone());
            }
        }
    }
    sfx
}

fn build_grid(
    size: Size<Tiles>,
    layers: &[RenderLayer],
    tiles: &TilePalette,
    obscuring: &[Rect<Tiles>],
) -> nav::Grid {
    let grid = size.grid();
    let cells = (grid.width * grid.height) as usize;
    let mut any_walkable = vec![false; cells];
    let mut any_blocked = vec![false; cells];

    for layer in layers {
        for (index, cell) in grid.cells().enumerate() {
            match tiles.walkable_of(layer.at(cell)) {
                Some(true) => any_walkable[index] = true,
                Some(false) => any_blocked[index] = true,
                None => {}
            }
        }
    }

    let mut walkable: Vec<bool> = (0..cells)
        .map(|i| any_walkable[i] && !any_blocked[i])
        .collect();
    for rect in obscuring {
        for c in obscured_cells(rect) {
            if let Some(i) = c.index(grid) {
                walkable[i] = false;
            }
        }
    }
    nav::Grid::new(grid, walkable)
}

fn obscured_cells(rect: &Rect<Tiles>) -> Vec<CellPos> {
    rect.tiles()
        .filter(|&c| cell_overlap(rect, c) >= OBSCURING_CUTOFF)
        .collect()
}
