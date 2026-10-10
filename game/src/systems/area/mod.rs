pub mod load;
pub mod transition;
pub mod warp;
pub mod zone;

pub use load::build_area;
pub use transition::Travel;
pub use warp::{WarpLock, lock_warps};
pub use zone::Zone;

use std::collections::{HashMap, HashSet};

use bevy_app::App;
use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;
use serde::{Deserialize, Serialize};

use crate::core::assets::{AssetRef, AssetService};
use crate::core::audio::playback::SfxId;
use crate::core::audio::soundscape::SoundscapeZone;
use crate::core::content::Content;
use crate::core::math::{Pos, Rect, Size};
use crate::core::tiling::{Cell, CellPos, GridSize, TilePos, TileSize, Tiles};
use crate::data;
use crate::systems::movement;
use crate::systems::notification::NotificationId;
use crate::systems::rule::Requirement;

pub use crate::data::area::Id;

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.replicate::<AreaTag>()
        .init_resource::<warp::WarpLocks>();
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AreaTag {
    pub area: Id,
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wild;

pub fn wild(world: &World, entity: Entity) -> bool {
    world.get::<Wild>(entity).is_some()
}

#[derive(Clone)]
pub struct AreaDef {
    pub name: &'static str,
    pub map: AssetRef,
    pub populations: &'static [Population],
    pub residents: &'static [Resident],
    pub props: &'static [crate::systems::prop::Fixture],
    pub zones: &'static [Zone],
    pub intro: Option<NotificationId>,
}

impl crate::core::content::ContentRow for AreaDef {
    const TABLE: &'static str = "area";
}

pub struct Population {
    pub npc: data::npc::Id,
    pub count: u32,
    pub roams: Option<MarkerName>,
}

pub struct Resident {
    pub npc: data::npc::Id,
    pub at: MarkerName,
    pub shown: &'static [&'static dyn Requirement],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MarkerName(pub &'static str);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MapMarker {
    Point(Pos<Tiles>),
    Rect(Rect<Tiles>),
}

impl MapMarker {
    pub fn center(self) -> Pos<Tiles> {
        match self {
            MapMarker::Point(at) => at,
            MapMarker::Rect(rect) => rect.center(),
        }
    }

    pub fn covers(self, at: Pos<Tiles>) -> bool {
        match self {
            MapMarker::Point(point) => point.cell() == at.cell(),
            MapMarker::Rect(rect) => rect.contains(at),
        }
    }
}

pub struct StandingOn(pub MarkerName);

impl Requirement for StandingOn {
    fn met(&self, world: &World, player: Entity) -> bool {
        let Some(at) = movement::position(world, player) else {
            return false;
        };
        !movement::moving(world, player)
            && of(world, player)
                .and_then(|area| area.marker(self.0))
                .is_some_and(|marker| marker.covers(at))
    }

    fn describe(&self, _content: &Content) -> String {
        format!("Standing on {}", self.0.0)
    }
}

pub fn walkable(world: &World, tile: Pos<Tiles>) -> bool {
    crate::systems::player::session::my_character(world)
        .map(|me| me.id())
        .and_then(|entity| of(world, entity))
        .is_some_and(|area| area.grid.walkable(tile))
}

#[derive(Clone)]
pub struct Portal {
    pub name: String,
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
    pub grid: movement::Grid,
    pub tile_sfx: Vec<Option<SfxId>>,
    pub wild_grid: movement::Grid,
    pub airspace: movement::Grid,
    pub wild_airspace: movement::Grid,
    pub safe_zones: Vec<Rect<Tiles>>,
    pub spawn: Pos<Tiles>,
    pub portals: Vec<Portal>,
    pub obscuring_rects: Vec<Rect<Tiles>>,
    pub groups: Vec<Group>,
    pub grouped_cells: HashSet<CellPos>,
    pub layers: Vec<RenderLayer>,
    pub markers: HashMap<String, MapMarker>,
    pub soundscape: Vec<SoundscapeZone>,
    pub map: std::sync::Arc<tiled::Map>,
}

impl Area {
    pub fn marker(&self, name: MarkerName) -> Option<MapMarker> {
        self.markers.get(name.0).copied()
    }

    pub fn range(&self, name: MarkerName) -> Option<Rect<Tiles>> {
        match self.marker(name)? {
            MapMarker::Rect(rect) => Some(rect),
            MapMarker::Point(_) => None,
        }
    }

    pub fn safe(&self, at: Pos<Tiles>) -> bool {
        let center = at.cell().center();
        self.safe_zones.iter().any(|zone| zone.contains(center))
    }

    pub fn grid_for(&self, wild: bool) -> &movement::Grid {
        if wild { &self.wild_grid } else { &self.grid }
    }

    pub fn airspace_for(&self, wild: bool) -> &movement::Grid {
        if wild {
            &self.wild_airspace
        } else {
            &self.airspace
        }
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
        let i = c.index(self.size.grid())?;
        self.tile_sfx[i].as_ref()
    }
}

pub fn check(assets: &AssetService) {
    let content = assets.content();
    let table = content.table::<AreaDef>();
    let areas = assets.resolve_all(table.rows().iter().map(|def| def.map), build_area);
    for (id, area) in table.ids().zip(areas) {
        let def = id.get(content);
        if !def.populations.is_empty() && area.wild_grid.nodes().is_empty() {
            panic!("area {id:?}: its populations have no ground outside the safe zones");
        }
        for population in def.populations {
            let Some(name) = population.roams else {
                continue;
            };
            let Some(bounds) = area.range(name) else {
                panic!(
                    "area {id:?}: {:?} roams '{}', which the map lacks as an area marker",
                    population.npc, name.0
                );
            };
            if !area
                .wild_grid
                .nodes()
                .iter()
                .any(|&node| bounds.contains(node))
            {
                panic!(
                    "area {id:?}: {:?} roams '{}', which has no ground outside the safe zones",
                    population.npc, name.0
                );
            }
        }
        for resident in def.residents {
            if area.marker(resident.at).is_none() {
                panic!(
                    "area {id:?}: {:?} stands on marker '{}', which the map lacks",
                    resident.npc, resident.at.0
                );
            }
        }
        for fixture in def.props {
            if area.marker(fixture.at).is_none() {
                panic!(
                    "area {id:?}: {:?} stands on marker '{}', which the map lacks",
                    fixture.prop, fixture.at.0
                );
            }
        }
        for zone in def.zones {
            if area.marker(zone.at).is_none() {
                panic!(
                    "area {id:?}: a zone covers marker '{}', which the map lacks",
                    zone.at.0
                );
            }
            if let Some(npc) = zone.with
                && !def.residents.iter().any(|resident| resident.npc == npc)
            {
                panic!("area {id:?}: a zone speaks for {npc:?}, who doesn't live here");
            }
            for outcome in zone.then {
                outcome.check(assets);
            }
        }
        for layer in area
            .soundscape
            .iter()
            .flat_map(|zone| zone.channels.values())
        {
            if let Err(error) = assets.open(std::path::Path::new(&layer.src)) {
                panic!("area {id:?}: soundscape track {}: {error}", layer.src);
            }
        }
    }
}

pub fn conversation_starts(content: &Content) -> Vec<data::dialogue::Id> {
    content
        .table::<AreaDef>()
        .rows()
        .iter()
        .flat_map(|def| def.zones)
        .flat_map(|zone| zone.then)
        .flat_map(|outcome| outcome.leads_to(content))
        .collect()
}

pub fn load(assets: &AssetService, area: Id) -> &'static Area {
    assets.resolve(area.get(assets.content()).map, build_area)
}

pub fn destination(assets: &AssetService, area: Id, at: MarkerName) -> Option<Pos<Tiles>> {
    let map = load(assets, area);
    let spot = map.marker(at)?.center();
    map.grid.walkable(spot).then_some(spot)
}

pub fn of(world: &World, entity: Entity) -> Option<&'static Area> {
    let area = world.get::<AreaTag>(entity)?.area;
    Some(load(world.resource::<AssetService>(), area))
}

fn cell_overlap(rect: &Rect<Tiles>, c: CellPos) -> f32 {
    rect.intersection(&c.bounds())
        .map_or(0.0, |overlap| overlap.area())
}
