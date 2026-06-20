use std::collections::HashSet;
use std::sync::OnceLock;

use serde::{Deserialize, Deserializer};
use tiled::{LayerType, PropertyValue};

use crate::actors::SfxId;
use crate::assets;
use crate::math::{Offset, Pos, Rect, Size, WorldPx};
use crate::nav;
use crate::table::{self, Content, Id};
use crate::tiling::{self, CellPos, PixelsPerTile, Tiles};
use crate::time::{Millis, Seconds};

const FILE: &str = "area_table.json";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AreaDef {
    pub id: String,
    pub map: MapRef,
    pub spawn: Option<bool>,
}

impl Content for AreaDef {
    fn table() -> &'static [AreaDef] {
        defs()
    }
    fn id(&self) -> &str {
        &self.id
    }
}

pub struct MapRef(pub String);

impl<'de> Deserialize<'de> for MapRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        if assets::find(assets::MAPS, &format!("{name}.tmx")).is_none() {
            return Err(serde::de::Error::custom(format!("unknown map '{name}'")));
        }
        Ok(MapRef(name))
    }
}

#[derive(Clone)]
pub struct Portal {
    pub rect: Rect<Tiles>,
    pub dest_area: Id<AreaDef>,
    pub dest: Pos<Tiles>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct TileRef(u32);

const FLIP_H: u32 = 0x8000_0000;
const FLIP_V: u32 = 0x4000_0000;
const FLIP_MASK: u32 = FLIP_H | FLIP_V;

impl TileRef {
    const EMPTY: TileRef = TileRef(0);

    fn new(index: usize, flip: (bool, bool)) -> TileRef {
        let mut bits = index as u32 + 1;
        if flip.0 {
            bits |= FLIP_H;
        }
        if flip.1 {
            bits |= FLIP_V;
        }
        TileRef(bits)
    }

    fn index(self) -> Option<usize> {
        match self.0 & !FLIP_MASK {
            0 => None,
            index => Some(index as usize - 1),
        }
    }

    fn flip(self) -> (bool, bool) {
        (self.0 & FLIP_H != 0, self.0 & FLIP_V != 0)
    }
}

#[derive(Clone)]
pub struct RenderLayer {
    pub dynamic: bool,
    width: i32,
    height: i32,
    cells: Vec<TileRef>,
}

impl RenderLayer {
    pub fn at(&self, c: CellPos) -> TileRef {
        tiling::cell_index(c, self.width, self.height).map_or(TileRef::EMPTY, |i| self.cells[i])
    }
}

#[derive(Clone)]
struct TileDef {
    sheet: String,
    frames: Vec<(Rect<WorldPx>, Millis)>,
    total: Millis,
}

#[derive(Clone)]
pub struct Group {
    pub bottom: Tiles,
    pub tiles: Vec<(CellPos, TileRef)>,
}

#[derive(Clone)]
pub struct Area {
    pub id: Id<AreaDef>,
    pub name: String,
    pub width: Tiles,
    pub height: Tiles,

    pub grid: nav::Grid,
    pub tile_sfx: Vec<Option<SfxId>>,
    pub spawn: Pos<Tiles>,
    pub portals: Vec<Portal>,

    pub walkable_nodes: Vec<Pos<Tiles>>,

    pub obscuring_rects: Vec<Rect<Tiles>>,

    pub objects: Vec<(Pos<Tiles>, TileRef)>,

    pub groups: Vec<Group>,

    pub grouped_cells: HashSet<CellPos>,

    pub layers: Vec<RenderLayer>,
    tiles: Vec<TileDef>,
}

pub struct TileSprite<'a> {
    pub sheet: &'a str,
    pub region: Rect<WorldPx>,
    pub flip: (bool, bool),
}

impl Area {
    pub fn resolve(&self, cell: TileRef, time: Seconds) -> Option<TileSprite<'_>> {
        let def = &self.tiles[cell.index()?];
        let region = if def.total.0 == 0.0 {
            def.frames[0].0
        } else {
            let mut remaining = time.millis().max(Millis(0.0)) % def.total;
            let mut region = def.frames[0].0;
            for &(frame, duration) in &def.frames {
                if remaining < duration {
                    region = frame;
                    break;
                }
                remaining -= duration;
            }
            region
        };
        Some(TileSprite {
            sheet: &def.sheet,
            region,
            flip: cell.flip(),
        })
    }

    pub fn animated(&self, cell: TileRef) -> bool {
        cell.index()
            .is_some_and(|index| self.tiles[index].total.0 > 0.0)
    }

    pub fn obscured_amount(&self, c: CellPos) -> f32 {
        self.obscuring_rects
            .iter()
            .map(|rect| cell_overlap(rect, c))
            .fold(0.0, f32::max)
    }

    pub fn dynamic_layer(&self) -> usize {
        self.layers
            .iter()
            .position(|layer| layer.dynamic)
            .expect("validated at load: every map has a 'Dynamic' layer")
    }

    pub fn tile_sfx_at(&self, c: CellPos) -> Option<&SfxId> {
        let i = tiling::cell_index(c, self.width.0 as i32, self.height.0 as i32)?;
        self.tile_sfx[i].as_ref()
    }
}

static AREA_COUNT: OnceLock<usize> = OnceLock::new();

pub fn configure_areas(count: usize) {
    let _ = AREA_COUNT.set(count);
}

pub fn areas() -> &'static [Area] {
    static AREAS: OnceLock<Vec<Area>> = OnceLock::new();
    AREAS.get_or_init(|| {
        let defs = defs();
        let base = defs.len() as u32;
        let count = AREA_COUNT.get().copied().unwrap_or(0).max(defs.len());
        let mut areas: Vec<Area> = defs
            .iter()
            .enumerate()
            .map(|(id, def)| build_area(Id::new(id as u32), &def.id, &def.map.0))
            .collect();
        for id in base..count as u32 {
            let mut clone = areas[(id % base) as usize].clone();
            clone.id = Id::new(id);
            clone.portals.clear();
            areas.push(clone);
        }
        areas
    })
}

pub fn defs() -> &'static [AreaDef] {
    static DEFS: OnceLock<Vec<AreaDef>> = OnceLock::new();
    DEFS.get_or_init(|| {
        let defs: Vec<AreaDef> = table::load(FILE);
        table::unique_ids(defs.iter().map(|def| def.id.as_str()), FILE);
        match defs.iter().filter(|def| def.spawn == Some(true)).count() {
            1 => {}
            n => panic!("{FILE}: exactly one area must set \"spawn\": true, found {n}"),
        }
        defs
    })
}

pub fn spawn_zone() -> Id<AreaDef> {
    let index = defs()
        .iter()
        .position(|def| def.spawn == Some(true))
        .expect("defs() validates exactly one spawn area");
    Id::new(index as u32)
}

fn load_map(name: &str) -> tiled::Map {
    tiled::Loader::with_reader(assets::tiled_reader)
        .load_tmx_map(format!("{}/{name}.tmx", assets::MAPS))
        .unwrap_or_else(|error| panic!("map '{name}': {error}"))
}

fn build_area(id: Id<AreaDef>, name: &str, map_name: &str) -> Area {
    let map = load_map(map_name);
    let tiling = PixelsPerTile::new(
        WorldPx(map.tile_width as f32),
        WorldPx(map.tile_height as f32),
    );
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
                let (width, height) = (map.width as i32, map.height as i32);
                let mut cells = vec![TileRef::EMPTY; (width * height) as usize];
                for (i, cell) in tiling::grid_cells(width, height).enumerate() {
                    if let Some(tile) = tile_layer.get_tile(cell.x, cell.y) {
                        cells[i] =
                            tiles.add(tile.get_tileset(), tile.id(), (tile.flip_h, tile.flip_v));
                    }
                }
                layers.push(RenderLayer {
                    dynamic: layer.name.eq_ignore_ascii_case("Dynamic"),
                    width,
                    height,
                    cells,
                });
            }
            LayerType::Objects(object_layer) => {
                for object in object_layer.objects() {
                    let pos = Pos::new(object.x, object.y);
                    if let Some(object_tile) = object.get_tile() {
                        let data = object.tile_data().expect("tile object has tile data");
                        let tileset = object_tile.get_tileset();
                        let cell = tiles.add(tileset, data.id(), (data.flip_h, data.flip_v));
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
    let walkable_nodes = tiling::grid_cells(map.width as i32, map.height as i32)
        .map(tiling::tile_center)
        .filter(|&p| grid.walkable(p))
        .collect();
    let (groups, grouped_cells) = compute_groups(&layers, &tiles);
    let tile_sfx = tile_sfx(size, &layers, &tiles);

    Area {
        id,
        name: name.to_owned(),
        width: Tiles(size.width),
        height: Tiles(size.height),
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

#[derive(Default)]
struct TilePalette {
    keys: Vec<(usize, u32)>,
    defs: Vec<TileDef>,
    walkable: Vec<Option<bool>>,
    group: Vec<Option<i64>>,
    sfx: Vec<Option<SfxId>>,
}

impl TilePalette {
    fn add(&mut self, tileset: &tiled::Tileset, id: u32, flip: (bool, bool)) -> TileRef {
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
    let (width, height) = (dynamic.width, dynamic.height);
    let group_at = |c: CellPos| -> Option<i64> { tiles.group_of(dynamic.at(c)) };

    let mut visited: HashSet<CellPos> = HashSet::new();
    for start in tiling::grid_cells(width, height) {
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
                let next = tiling::cell_step(c, step);
                if group_at(next) == Some(group_id) && visited.insert(next) {
                    stack.push(next);
                }
            }
        }
        groups.push(Group {
            // Depth sorts against actor/object centers (see tiling.rs); anchoring at the
            // bottom row's near edge keeps a player on that row in front, never tying.
            bottom: Tiles(tiling::tile_bounds(CellPos::new(0, bottom)).min().y),
            tiles: cells,
        });
    }
    (groups, grouped_cells)
}

fn tile_sfx(size: Size<Tiles>, layers: &[RenderLayer], tiles: &TilePalette) -> Vec<Option<SfxId>> {
    let (width, height) = (size.width as i32, size.height as i32);
    let mut sfx = vec![None; (width * height) as usize];
    for layer in layers {
        for (i, cell) in tiling::grid_cells(width, height).enumerate() {
            if let Some(id) = tiles.sfx_of(layer.at(cell)) {
                sfx[i] = Some(id.clone());
            }
        }
    }
    sfx
}

const OBSCURING_CUTOFF: f32 = 0.4;

fn build_grid(
    size: Size<Tiles>,
    layers: &[RenderLayer],
    tiles: &TilePalette,
    obscuring: &[Rect<Tiles>],
) -> nav::Grid {
    let (width, height) = (size.width as i32, size.height as i32);
    let cells = (width * height) as usize;
    let mut any_walkable = vec![false; cells];
    let mut any_blocked = vec![false; cells];

    for layer in layers {
        for (index, cell) in tiling::grid_cells(width, height).enumerate() {
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
            if let Some(i) = tiling::cell_index(c, width, height) {
                walkable[i] = false;
            }
        }
    }
    nav::Grid::new(size, walkable)
}

fn obscured_cells(rect: &Rect<Tiles>) -> Vec<CellPos> {
    tiling::tiles_in(*rect)
        .filter(|&c| cell_overlap(rect, c) >= OBSCURING_CUTOFF)
        .collect()
}

fn cell_overlap(rect: &Rect<Tiles>, c: CellPos) -> f32 {
    rect.intersection(&tiling::tile_bounds(c))
        .map_or(0.0, |overlap| overlap.area())
}
