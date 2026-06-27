pub mod load;
pub mod transition;

pub use load::build_area;

use std::collections::HashSet;

use bevy_app::App;
use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;
use serde::{Deserialize, Serialize};

use crate::core::assets::{AssetRef, AssetService};
use crate::core::math::{Pos, Rect, Size};
use crate::core::nav;
use crate::core::sfx::SfxId;
use crate::core::tiling::{Cell, CellPos, GridSize, TileSize, Tiles};
use crate::data;

pub use crate::data::area::Id;

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.replicate::<AreaTag>();
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AreaTag {
    pub area: Id,
}

pub struct AreaDef {
    pub map: AssetRef,
    pub bench: bool,
    pub spawns: &'static [Spawn],
}

pub struct Spawn {
    pub npc: data::npc::Id,
    pub population: u32,
}

pub fn walkable(world: &World, tile: Pos<Tiles>) -> bool {
    crate::systems::player::session::me(world)
        .map(|me| me.id())
        .and_then(|entity| of(world, entity))
        .is_some_and(|area| area.grid.walkable(tile))
}

#[derive(Clone)]
pub struct Portal {
    pub rect: Rect<Tiles>,
    pub dest_area: Id,
    pub dest: Pos<Tiles>,
}

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

pub fn of(world: &World, entity: Entity) -> Option<&'static Area> {
    let map = world.get::<AreaTag>(entity)?.area.get().map;
    Some(world.resource::<AssetService>().resolve(map, build_area))
}

fn cell_overlap(rect: &Rect<Tiles>, c: CellPos) -> f32 {
    rect.intersection(&c.bounds())
        .map_or(0.0, |overlap| overlap.area())
}
