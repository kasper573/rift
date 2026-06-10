use std::collections::HashSet;
use std::io::Cursor;
use std::path::Path;
use std::sync::OnceLock;

use rift::Wire;
use serde::{Deserialize, Deserializer};
use tiled::{LayerType, PropertyValue};

use crate::core::actors::SfxId;
use crate::core::assets;
use crate::core::math::{Pixels, Pos, Rect, Size, Tiles, Tiling};
use crate::core::table;

const FILE: &str = "area_table.json";

/// An area's index in [`areas`]; content tables reference areas by id.
#[derive(Wire, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct AreaId(pub u32);

impl<'de> Deserialize<'de> for AreaId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let id = String::deserialize(deserializer)?;
        defs()
            .iter()
            .position(|def| def.id == id)
            .map(|index| AreaId(index as u32))
            .ok_or_else(|| serde::de::Error::custom(format!("unknown area '{id}'")))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AreaDef {
    pub id: String,
    pub map: MapRef,
    pub spawn: Option<bool>,
}

/// A map asset's name; parsing validates that the map exists.
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
    pub dest_area: AreaId,
    pub dest: Pos<Tiles>,
}

/// One map tile reference in a layer cell or a placed object: a [`TileDef`] index plus flips;
/// zero is the empty cell.
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

/// One tile layer, in map order, ready to draw: `dynamic` marks the layer whose grouped cells
/// render z-sorted instead.
#[derive(Clone)]
pub struct RenderLayer {
    pub dynamic: bool,
    width: i32,
    height: i32,
    cells: Vec<TileRef>,
}

impl RenderLayer {
    /// The cell at (x, y); empty outside the layer.
    pub fn at(&self, x: i32, y: i32) -> TileRef {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return TileRef::EMPTY;
        }
        self.cells[(y * self.width + x) as usize]
    }
}

/// One distinct map tile's render data: its tileset sheet and its animation frames (a static
/// tile is a single endless frame).
#[derive(Clone)]
struct TileDef {
    sheet: &'static [u8],
    frames: Vec<(Rect<Pixels>, u32)>,
    total_ms: u32,
}

#[derive(Clone)]
pub struct Group {
    pub z: f32,
    pub tiles: Vec<(i32, i32, TileRef)>,
}

#[derive(Clone)]
pub struct Area {
    pub id: AreaId,
    pub name: String,
    pub width: Tiles,
    pub height: Tiles,

    pub grid: nav::Grid<Tiles>,
    pub tile_sfx: Vec<Option<SfxId>>,
    pub spawn: Pos<Tiles>,
    pub portals: Vec<Portal>,

    pub walkable_nodes: Vec<Pos<Tiles>>,

    pub obscuring_rects: Vec<Rect<Tiles>>,

    pub objects: Vec<(Pos<Tiles>, TileRef)>,

    pub groups: Vec<Group>,

    pub grouped_cells: HashSet<(i32, i32)>,

    pub layers: Vec<RenderLayer>,
    tiles: Vec<TileDef>,
}

/// What to draw for one resolved map cell: a region of a tileset sheet, possibly mirrored.
pub struct TileSprite {
    pub sheet: &'static [u8],
    pub region: Rect<Pixels>,
    pub flip: (bool, bool),
}

impl Area {
    /// The sprite to draw for a cell at time `t`; animated tiles advance.
    pub fn resolve(&self, cell: TileRef, time: f32) -> Option<TileSprite> {
        let def = &self.tiles[cell.index()?];
        let region = if def.total_ms == 0 {
            def.frames[0].0
        } else {
            let mut remaining = (time * 1000.0).max(0.0) as u32 % def.total_ms;
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
            sheet: def.sheet,
            region,
            flip: cell.flip(),
        })
    }

    /// The largest fraction of the tile cell at (x, y) covered by any obscuring object.
    pub fn obscured_amount(&self, x: i32, y: i32) -> f32 {
        self.obscuring_rects
            .iter()
            .map(|rect| cell_overlap(rect, x, y))
            .fold(0.0, f32::max)
    }

    /// The footstep sfx of the tile at (x, y), if its tileset declares one.
    pub fn tile_sfx_at(&self, x: i32, y: i32) -> Option<&SfxId> {
        if x < 0 || y < 0 || x >= self.width.0 as i32 || y >= self.height.0 as i32 {
            return None;
        }
        self.tile_sfx[(y * self.width.0 as i32 + x) as usize].as_ref()
    }
}

static AREA_COUNT: OnceLock<usize> = OnceLock::new();

/// Must run before [`areas`] is first used; extra areas are portal-less clones of the base
/// areas so benchmarks can scatter entities across a larger world.
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
            .map(|(id, def)| build_area(AreaId(id as u32), &def.id, &def.map.0))
            .collect();
        for id in base..count as u32 {
            let mut clone = areas[(id % base) as usize].clone();
            clone.id = AreaId(id);
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

pub fn spawn_zone() -> AreaId {
    let index = defs()
        .iter()
        .position(|def| def.spawn == Some(true))
        .expect("defs() validates exactly one spawn area");
    AreaId(index as u32)
}

/// Resolves an area id from map data; `context` names the referencing map on panic.
fn index(id: &str, context: &str) -> AreaId {
    let index = defs()
        .iter()
        .position(|def| def.id == id)
        .unwrap_or_else(|| panic!("{context}: unknown area '{id}'"));
    AreaId(index as u32)
}

/// Reads `maps/<name>.tmx` and everything it references out of the embedded assets.
fn load_map(name: &str) -> tiled::Map {
    let mut loader = tiled::Loader::with_reader(|path: &Path| -> std::io::Result<_> {
        let key = embedded_key(path);
        assets::bytes(&key)
            .map(Cursor::new)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, key))
    });
    loader
        .load_tmx_map(format!("{}/{name}.tmx", assets::MAPS))
        .unwrap_or_else(|error| panic!("map '{name}': {error}"))
}

/// Normalizes the loader's relative paths (`maps/../tilesets/x.tsx`) to embedded asset keys.
fn embedded_key(path: &Path) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => {
                parts.pop();
            }
            std::path::Component::Normal(name) => {
                parts.push(name.to_str().unwrap_or_default());
            }
            _ => {}
        }
    }
    parts.join("/")
}

fn build_area(id: AreaId, name: &str, map_name: &str) -> Area {
    let map = load_map(map_name);
    let tiling = Tiling::new(
        Pixels(map.tile_width as f32),
        Pixels(map.tile_height as f32),
    );
    let size = Size::new(Tiles(map.width as f32), Tiles(map.height as f32));

    let mut tiles = TileTable::default();
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
                for y in 0..height {
                    for x in 0..width {
                        if let Some(cell) = tile_layer.get_tile(x, y) {
                            cells[(y * width + x) as usize] = tiles.intern(
                                cell.get_tileset(),
                                cell.id(),
                                (cell.flip_h, cell.flip_v),
                            );
                        }
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
                    let pos = Pos::new(Pixels(object.x), Pixels(object.y));
                    if let Some(object_tile) = object.get_tile() {
                        let data = object.tile_data().expect("tile object has tile data");
                        let tileset = object_tile.get_tileset();
                        let cell = tiles.intern(tileset, data.id(), (data.flip_h, data.flip_v));
                        objects.push((tiling.point(pos), cell));
                        // Tiled anchors tile objects at their bottom-left corner. Only tiles the
                        // tileset declares something about can opt out of obscuring.
                        let obscuring = object_tile.get_tile().is_some_and(|tile| {
                            tile.properties.get("Walkable") != Some(&PropertyValue::BoolValue(true))
                        });
                        if obscuring {
                            let object_size = shape_size(&object.shape);
                            obscuring_rects.push(tiling.rect(Rect::new(
                                Pos::new(pos.x, pos.y - object_size.y),
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
    let grid = build_grid(size, &layers, &tiles, &obscuring_rects);
    let walkable_nodes = (0..map.height as i32)
        .flat_map(|y| {
            (0..map.width as i32).map(move |x| Pos::new(Tiles(x as f32), Tiles(y as f32)))
        })
        .filter(|&p| grid.walkable(p))
        .collect();
    let (groups, grouped_cells) = compute_groups(&layers, &tiles);
    let tile_sfx = tile_sfx(size, &layers, &tiles);

    Area {
        id,
        name: name.to_owned(),
        width: size.x,
        height: size.y,
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

/// The interned set of distinct map tiles, keyed by tileset identity and local tile id; per-tile
/// properties are captured alongside so grid building never re-touches the tiled crate.
#[derive(Default)]
struct TileTable {
    keys: Vec<(usize, u32)>,
    defs: Vec<TileDef>,
    walkable: Vec<Option<bool>>,
    group: Vec<Option<i64>>,
    sfx: Vec<Option<SfxId>>,
}

impl TileTable {
    fn intern(&mut self, tileset: &tiled::Tileset, id: u32, flip: (bool, bool)) -> TileRef {
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
        .map(|(_, bytes)| bytes)
        .unwrap_or_else(|| panic!("unknown tileset image {source}"));

    let region = |id: u32| {
        let columns = tileset.columns.max(1);
        Rect::new(
            Pos::new(
                Pixels(
                    (tileset.margin + (id % columns) * (tileset.tile_width + tileset.spacing))
                        as f32,
                ),
                Pixels(
                    (tileset.margin + (id / columns) * (tileset.tile_height + tileset.spacing))
                        as f32,
                ),
            ),
            Size::new(
                Pixels(tileset.tile_width as f32),
                Pixels(tileset.tile_height as f32),
            ),
        )
    };

    let animation = tileset.get_tile(id).and_then(|tile| tile.animation.clone());
    let frames: Vec<(Rect<Pixels>, u32)> = match animation {
        Some(frames) if !frames.is_empty() => frames
            .iter()
            .map(|frame| (region(frame.tile_id), frame.duration))
            .collect(),
        _ => vec![(region(id), 0)],
    };
    let total_ms = frames.iter().map(|&(_, duration)| duration).sum();
    TileDef {
        sheet,
        frames,
        total_ms,
    }
}

fn shape_size(shape: &tiled::ObjectShape) -> Size<Pixels> {
    match shape {
        tiled::ObjectShape::Rect { width, height }
        | tiled::ObjectShape::Ellipse { width, height } => {
            Size::new(Pixels(*width), Pixels(*height))
        }
        _ => Size::splat(Pixels(0.0)),
    }
}

/// A warp object's portal; every warp must carry a well-formed `goto: "<area>, x, y"`.
fn portal(
    name: &str,
    goto: &str,
    pos: Pos<Pixels>,
    shape: &tiled::ObjectShape,
    tiling: Tiling,
) -> Portal {
    let malformed = || -> ! { panic!("map '{name}': goto '{goto}' must be '<area>, x, y'") };
    let mut parts = goto.split(',');
    let dest = parts.next().unwrap_or_else(|| malformed()).trim();
    let dest_area = index(dest, &format!("map '{name}'"));
    let mut coord = || -> f32 {
        parts
            .next()
            .and_then(|part| part.trim().parse().ok())
            .unwrap_or_else(|| malformed())
    };
    Portal {
        rect: tiling.rect(Rect::new(pos, shape_size(shape))),
        dest_area,
        dest: Pos::new(Tiles(coord()), Tiles(coord())),
    }
}

fn compute_groups(layers: &[RenderLayer], tiles: &TileTable) -> (Vec<Group>, HashSet<(i32, i32)>) {
    let mut groups = Vec::new();
    let mut grouped_cells = HashSet::new();
    let Some(dynamic) = layers.iter().find(|layer| layer.dynamic) else {
        return (groups, grouped_cells);
    };
    let (width, height) = (dynamic.width, dynamic.height);
    let group_at = |x: i32, y: i32| -> Option<i64> { tiles.group_of(dynamic.at(x, y)) };

    let mut visited: HashSet<(i32, i32)> = HashSet::new();
    for sy in 0..height {
        for sx in 0..width {
            let Some(group_id) = group_at(sx, sy) else {
                continue;
            };
            if !visited.insert((sx, sy)) {
                continue;
            }
            let mut stack = vec![(sx, sy)];
            let mut cells = Vec::new();
            let mut bottom = sy;
            while let Some((x, y)) = stack.pop() {
                cells.push((x, y, dynamic.at(x, y)));
                grouped_cells.insert((x, y));
                bottom = bottom.max(y);
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let (nx, ny) = (x + dx, y + dy);
                    if group_at(nx, ny) == Some(group_id) && visited.insert((nx, ny)) {
                        stack.push((nx, ny));
                    }
                }
            }
            groups.push(Group {
                z: bottom as f32,
                tiles: cells,
            });
        }
    }
    (groups, grouped_cells)
}

// Per-cell footstep sfx, flattened row-major; the topmost tile layer that declares one wins.
fn tile_sfx(size: Size<Tiles>, layers: &[RenderLayer], tiles: &TileTable) -> Vec<Option<SfxId>> {
    let (width, height) = (size.x.0 as i32, size.y.0 as i32);
    let mut sfx = vec![None; (width * height) as usize];
    for layer in layers {
        for y in 0..height {
            for x in 0..width {
                if let Some(id) = tiles.sfx_of(layer.at(x, y)) {
                    sfx[(y * width + x) as usize] = Some(id.clone());
                }
            }
        }
    }
    sfx
}

/// A tile object blocks every cell it covers by at least this fraction
const OBSCURING_CUTOFF: f32 = 0.4;

fn build_grid(
    size: Size<Tiles>,
    layers: &[RenderLayer],
    tiles: &TileTable,
    obscuring: &[Rect<Tiles>],
) -> nav::Grid<Tiles> {
    let (width, height) = (size.x.0 as i32, size.y.0 as i32);
    let cells = (width * height) as usize;
    let mut any_walkable = vec![false; cells];
    let mut any_blocked = vec![false; cells];

    for layer in layers {
        for y in 0..height {
            for x in 0..width {
                let index = (y * width + x) as usize;
                match tiles.walkable_of(layer.at(x, y)) {
                    Some(true) => any_walkable[index] = true,
                    Some(false) => any_blocked[index] = true,
                    None => {}
                }
            }
        }
    }

    let mut walkable: Vec<bool> = (0..cells)
        .map(|i| any_walkable[i] && !any_blocked[i])
        .collect();
    for rect in obscuring {
        for c in obscured_cells(rect) {
            if c.x >= 0 && c.y >= 0 && c.x < width && c.y < height {
                walkable[(c.y * width + c.x) as usize] = false;
            }
        }
    }
    nav::Grid::new(size, walkable)
}

fn obscured_cells(rect: &Rect<Tiles>) -> Vec<Pos<i32>> {
    let mut cells = Vec::new();
    for y in (rect.pos.y.0.floor() as i32)..((rect.pos.y.0 + rect.size.y.0).ceil() as i32) {
        for x in (rect.pos.x.0.floor() as i32)..((rect.pos.x.0 + rect.size.x.0).ceil() as i32) {
            if cell_overlap(rect, x, y) >= OBSCURING_CUTOFF {
                cells.push(Pos::new(x, y));
            }
        }
    }
    cells
}

/// The fraction of the 1x1 tile cell at (x, y) covered by `rect`.
fn cell_overlap(rect: &Rect<Tiles>, x: i32, y: i32) -> f32 {
    let cell = Rect::new(
        Pos::new(Tiles(x as f32), Tiles(y as f32)),
        Size::splat(Tiles(1.0)),
    );
    rect.intersection(cell)
        .map_or(0.0, |overlap| overlap.area())
}
