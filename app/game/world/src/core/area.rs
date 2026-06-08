use std::collections::HashSet;
use std::sync::OnceLock;

use actor::SfxId;
use rift::Wire;
use serde::{Deserialize, Deserializer};

use crate::core::assets;
use crate::core::math::{Pos, Rect, Size, Tiles, Tiling};
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
        if assets::find_text(assets::MAPS, &format!("{name}.json")).is_none() {
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

#[derive(Clone)]
pub struct Area {
    pub id: AreaId,
    pub name: String,
    pub width: Tiles,
    pub height: Tiles,
    pub map: tiled::Map,

    pub tilesets: tiled::Tilesets,
    pub grid: nav::Grid<Tiles>,
    pub tile_sfx: Vec<Option<SfxId>>,
    pub spawn: Pos<Tiles>,
    pub portals: Vec<Portal>,

    pub walkable_nodes: Vec<Pos<Tiles>>,

    pub obscuring_rects: Vec<Rect<Tiles>>,

    pub objects: Vec<(Pos<Tiles>, u32)>,

    pub groups: Vec<Group>,

    pub grouped_cells: HashSet<(i32, i32)>,
}

impl Area {
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

#[derive(Clone)]
pub struct Group {
    pub z: f32,

    pub tiles: Vec<(i32, i32, u32)>,
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
            .map(|(id, def)| {
                let json = assets::find_text(assets::MAPS, &format!("{}.json", def.map.0))
                    .expect("MapRef validates every map reference");
                build_area(AreaId(id as u32), &def.id, json)
            })
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

fn build_area(id: AreaId, name: &str, map_json: &str) -> Area {
    let map = tiled::load_map(map_json).expect("embedded Tiled map should parse");
    let tilesets = tiled::Tilesets::load(&map, |path| {
        assets::find(assets::TILESETS, path).map(|(_, bytes)| bytes)
    })
    .unwrap_or_else(|error| panic!("area {name}: {error}"));

    let tiling = Tiling::new(map.tile_width, map.tile_height);
    let obscuring_rects = obscuring_rects(&map, &tilesets, tiling);
    let grid = build_grid(&map, &tilesets, &obscuring_rects);
    let walkable_nodes = (0..map.height.0 as i32)
        .flat_map(|y| {
            (0..map.width.0 as i32).map(move |x| Pos::new(Tiles(x as f32), Tiles(y as f32)))
        })
        .filter(|&p| grid.walkable(p))
        .collect();
    let objects = map
        .objects()
        .iter()
        .filter(|object| object.gid != 0)
        .map(|object| (tiling.point(object.pos), object.gid))
        .collect();
    let spawn = start(name, &map, tiling);
    let portals = portals(name, &map, tiling);
    let (groups, grouped_cells) = compute_groups(&map, &tilesets);
    let tile_sfx = tile_sfx(&map, &tilesets);

    Area {
        id,
        name: name.to_owned(),
        width: map.width,
        height: map.height,
        map,
        tilesets,
        grid,
        tile_sfx,
        spawn,
        portals,
        walkable_nodes,
        obscuring_rects,
        objects,
        groups,
        grouped_cells,
    }
}

/// The map's start object, snapped to its tile's center; every map must place one.
fn start(name: &str, map: &tiled::Map, tiling: Tiling) -> Pos<Tiles> {
    map.objects()
        .iter()
        .find(|object| object.kind == "start")
        .map(|start| tiling.tile_center(start.pos))
        .unwrap_or_else(|| panic!("map '{name}' must place a start object"))
}

/// The map's warp objects as portals; every warp must carry a well-formed
/// `goto: "<area>, x, y"` referencing a known area.
fn portals(name: &str, map: &tiled::Map, tiling: Tiling) -> Vec<Portal> {
    map.objects()
        .iter()
        .filter(|object| object.kind == "warp")
        .map(|warp| {
            let goto = warp
                .properties
                .get("goto")
                .and_then(tiled::Prop::as_str)
                .unwrap_or_else(|| panic!("map '{name}': warp {} lacks a goto property", warp.id));
            let malformed =
                || -> ! { panic!("map '{name}': goto '{goto}' must be '<area>, x, y'") };
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
                rect: tiling.rect(Rect::new(warp.pos, warp.size)),
                dest_area,
                dest: Pos::new(Tiles(coord()), Tiles(coord())),
            }
        })
        .collect()
}

fn compute_groups(
    map: &tiled::Map,
    tilesets: &tiled::Tilesets,
) -> (Vec<Group>, HashSet<(i32, i32)>) {
    let mut groups = Vec::new();
    let mut grouped_cells = HashSet::new();
    let Some(dynamic) = map.tile_layer("Dynamic") else {
        return (groups, grouped_cells);
    };
    let (width, height) = (dynamic.width as i32, dynamic.height as i32);
    let group_at = |x: i32, y: i32| -> Option<i64> { tile_group(tilesets, dynamic.at(x, y)) };

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
            let mut tiles = Vec::new();
            let mut bottom = sy;
            while let Some((x, y)) = stack.pop() {
                tiles.push((x, y, dynamic.at(x, y)));
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
                tiles,
            });
        }
    }
    (groups, grouped_cells)
}

fn tile_group(tilesets: &tiled::Tilesets, raw: u32) -> Option<i64> {
    match tilesets.property(raw, "Group")? {
        tiled::Prop::Int(n) => Some(*n),
        tiled::Prop::Float(f) => Some(*f as i64),
        tiled::Prop::Str(s) => s.parse().ok(),
        tiled::Prop::Bool(_) => None,
    }
}

// Per-cell footstep sfx, flattened row-major; the topmost tile layer that declares one wins.
fn tile_sfx(map: &tiled::Map, tilesets: &tiled::Tilesets) -> Vec<Option<SfxId>> {
    let width = map.width.0 as i32;
    let mut sfx = vec![None; (map.width.0 * map.height.0) as usize];
    for tiles in map.tile_layers() {
        for (x, y, raw) in tiles.cells() {
            if let Some(name) = tilesets.property(raw, "sfx").and_then(tiled::Prop::as_str) {
                sfx[(y * width + x) as usize] = Some(SfxId(name.to_owned()));
            }
        }
    }
    sfx
}

/// A tile object blocks every cell it covers by at least this fraction
const OBSCURING_CUTOFF: f32 = 0.4;

fn build_grid(
    map: &tiled::Map,
    tilesets: &tiled::Tilesets,
    obscuring: &[Rect<Tiles>],
) -> nav::Grid<Tiles> {
    let width = map.width.0 as i32;
    let height = map.height.0 as i32;
    let cells = (width * height) as usize;
    let mut any_walkable = vec![false; cells];
    let mut any_blocked = vec![false; cells];

    for tiles in map.tile_layers() {
        for (x, y, raw) in tiles.cells() {
            let index = (y * width + x) as usize;
            match tilesets.property(raw, "Walkable") {
                Some(tiled::Prop::Bool(true)) => any_walkable[index] = true,
                Some(tiled::Prop::Bool(false)) => any_blocked[index] = true,
                _ => {}
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
    nav::Grid::new(Size::new(map.width, map.height), walkable)
}

/// Tile objects whose tileset tile is not walkable, as rects in tile units.
/// Tiled anchors tile objects at their bottom-left corner.
fn obscuring_rects(
    map: &tiled::Map,
    tilesets: &tiled::Tilesets,
    tiling: Tiling,
) -> Vec<Rect<Tiles>> {
    map.objects()
        .iter()
        .filter(|object| object.gid != 0)
        .filter(|object| {
            tilesets.tile(object.gid).is_some_and(|tile| {
                tile.properties.get("Walkable") != Some(&tiled::Prop::Bool(true))
            })
        })
        .map(|object| {
            tiling.rect(Rect::new(
                Pos::new(object.pos.x, object.pos.y - object.size.y),
                object.size,
            ))
        })
        .collect()
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
