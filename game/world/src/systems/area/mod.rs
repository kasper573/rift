//! Areas: the runtime map an [`Area`] exposes — render layers, depth groups, nav grid, portals, and
//! tile sound — the [`AreaDef`] table, and the [`AreaTag`] marking which area an entity is in. Map
//! construction from Tiled lives in [`load`]; the cross-area player handoff in [`transition`].

pub mod load;
pub mod transition;

use std::collections::HashSet;
use std::sync::OnceLock;

use bevy_app::App;
use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;
use serde::{Deserialize, Deserializer, Serialize};

use crate::core::assets;
use crate::core::math::{Pos, Rect, Size};
use crate::core::nav;
use crate::core::table::{self, Content, Id};
use crate::core::tiling::{Cell, CellPos, GridSize, TileSize, Tiles};
use crate::systems::sfx::SfxId;

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;

    app.replicate::<AreaTag>();
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AreaTag {
    pub area: Id<AreaDef>,
}

/// Whether the local player can step onto `tile` in its current area — client-side path validation.
pub fn walkable(world: &World, tile: Pos<Tiles>) -> bool {
    crate::systems::player::session::me(world)
        .and_then(|me| me.get::<AreaTag>())
        .map(|tag| tag.area)
        .and_then(|id| areas().get(id.index()))
        .is_some_and(|area| area.grid.walkable(tile))
}

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

/// A cell's tile as an index into the area's tile palette (`0` is empty). Tile *rendering* reads the
/// raw `tiled::Map`; this index only resolves a cell's gameplay metadata (walkable/group/sfx), which a
/// horizontal/vertical flip never changes, so flips are not stored.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct TileRef(u32);

impl TileRef {
    const EMPTY: TileRef = TileRef(0);

    fn new(index: usize) -> TileRef {
        TileRef(index as u32 + 1)
    }

    fn index(self) -> Option<usize> {
        match self.0 {
            0 => None,
            index => Some(index as usize - 1),
        }
    }
}

#[derive(Clone)]
pub struct RenderLayer {
    pub dynamic: bool,
    size: GridSize,
    cells: Vec<TileRef>,
}

impl RenderLayer {
    pub fn at(&self, c: CellPos) -> TileRef {
        c.index(self.size).map_or(TileRef::EMPTY, |i| self.cells[i])
    }
}

#[derive(Clone)]
pub struct Group {
    pub bottom: Tiles,
    pub tiles: Vec<CellPos>,
}

#[derive(Clone)]
pub struct Area {
    pub id: Id<AreaDef>,
    pub name: String,
    pub size: Size<Tiles>,

    pub grid: nav::Grid,
    pub tile_sfx: Vec<Option<SfxId>>,
    pub spawn: Pos<Tiles>,
    pub portals: Vec<Portal>,

    pub walkable_nodes: Vec<Pos<Tiles>>,

    pub obscuring_rects: Vec<Rect<Tiles>>,

    pub groups: Vec<Group>,

    pub grouped_cells: HashSet<CellPos>,

    pub layers: Vec<RenderLayer>,

    pub map: std::sync::Arc<tiled::Map>,
}

impl Area {
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
        let i = c.index(self.size.grid())?;
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
            .map(|(id, def)| load::build_area(Id::new(id as u32), &def.id, &def.map.0))
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

/// The area with this id, or `None` if it is out of range.
pub fn get(id: Id<AreaDef>) -> Option<&'static Area> {
    areas().get(id.index())
}

/// The area an entity currently stands in, resolved from its [`AreaTag`].
pub fn of(world: &World, entity: Entity) -> Option<&'static Area> {
    get(world.get::<AreaTag>(entity)?.area)
}

/// Builds a one-off [`Area`] straight from an embedded map file, bypassing the [`AreaDef`] table —
/// for devtools that render an arbitrary map by name (the `render` preview binary). Panics the same
/// way the table path does if the map is missing or malformed.
pub fn preview(map_name: &str) -> Area {
    load::build_area(Id::new(0), map_name, map_name)
}

/// Like [`preview`] but reads the `.tmx` straight from a filesystem path, so devtools can render maps
/// that aren't in the embed or the area table — instant iteration with no rebuild.
pub fn preview_path(path: &std::path::Path) -> Area {
    let name = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("preview");
    load::build_from_map(Id::new(0), name, load::load_map_path(path))
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

fn cell_overlap(rect: &Rect<Tiles>, c: CellPos) -> f32 {
    rect.intersection(&c.bounds())
        .map_or(0.0, |overlap| overlap.area())
}
