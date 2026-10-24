pub mod blueprint;
pub mod grid;
pub mod object;
pub mod scene;
pub mod tileset;
pub mod transition;
pub mod walk;
pub mod warp;
pub mod world_map;

pub use blueprint::{BlueprintDef, Flattened, PlacedBlueprint, Placement};
pub use grid::{AreaGrids, Flip, PaletteEntry};
pub use object::{AreaObject, ObjectDef, ObjectShape};
pub use scene::{Scene, SceneNode};

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io;
use std::sync::Arc;

use bevy_app::App;
use bevy_ecs::component::Component;
use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{EnumString, IntoStaticStr, VariantArray};

use crate::core::assets::AudioRef;
use crate::core::audio::mix::AudioCategory;
use crate::core::audio::playback::SfxId;
use crate::core::audio::soundscape::{
    SoundscapeChannel, SoundscapeLayer, SoundscapeShape, SoundscapeZone,
};
use crate::core::content::media::ImageSizes;
use crate::core::content::{
    Bundle, Content, ContentFile, ContentId, ContentProblem, ContentRegistry, FileWriter,
    LoadError, LoadStep, Loading, NamedContent, Nearby, PartSpec, Parts, RowCheck, Shape, serial,
};
use crate::core::math::{Offset, Pos, Rect, Size};
use crate::core::render::dynamic_z;
use crate::core::units::{Cell, CellPos, GridSize, Meters, TilePos, TileRect, TileSize};
use crate::systems::area::tileset::TilesetId;
use crate::systems::movement;
use crate::systems::npc::NpcId;
use crate::systems::script::{self, Condition, OwnerRow, Reach, RiftScripts, ScriptModule};
use blueprint::FlattenAt;
use scene::{GroundLayer, SCENE_PARTS, SceneGrids, scene_file};

pub type AreaId = ContentId<AreaDef>;

pub const MAX_SIDE: u32 = 256;
pub const MIN_SIDE: u32 = 4;
pub const MAX_CELLS: u32 = 65_536;
pub const MAX_GROUND_LAYERS: usize = 16;
pub const MAX_POPULATION: u32 = 500;

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.replicate_once::<AreaTag>();
    object::register(app);
}

pub fn content(registry: &mut ContentRegistry) {
    registry
        .bundle::<tileset::TilesetDef>()
        .file::<tileset::SurfaceDef>()
        .named::<tileset::SurfaceDef>()
        .bundle::<BlueprintDef>()
        .bundle::<AreaDef>()
        .named::<AreaDef>()
        .file::<world_map::WorldMapDef>()
        .beside(grid::palette_beside(AreaDef::TABLE))
        .beside(grid::palette_beside(BlueprintDef::TABLE))
        .json_part(scene::json_part(AreaDef::TABLE))
        .json_part(scene::json_part(BlueprintDef::TABLE))
        .hook(LoadStep::Files, resolve_grids)
        .hook(LoadStep::Expand, flatten_all)
        .hook(LoadStep::Assemble, build_all)
        .hook(LoadStep::Across, world_map::check_all);
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AreaTag {
    pub area: AreaId,
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wild;

pub fn wild(world: &World, entity: Entity) -> bool {
    world.get::<Wild>(entity).is_some()
}

pub fn same_area(world: &World, a: Entity, b: Entity) -> bool {
    let area = |entity| world.get::<AreaTag>(entity).map(|tag| tag.area);
    area(a).is_some() && area(a) == area(b)
}

/// A place in the world: a blueprint with a name, whose own ground and modules are all it holds.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AreaDef {
    pub name: String,
    pub blueprint: BlueprintDef,
}

impl ContentFile for AreaDef {
    const TABLE: &'static str = "area";
    const SHAPE: Shape = Shape::Bundle;

    fn fresh(near: &Nearby) -> Result<AreaDef, String> {
        let ground = SceneNode::GroundLayer(GroundLayer::named(GroundLayer::FIRST));
        Ok(AreaDef {
            name: near.name(),
            blueprint: BlueprintDef {
                map: Tilemap::fresh(MapSize {
                    width: 16,
                    height: 12,
                }),
                variables: BTreeMap::new(),
                scene: Scene {
                    nodes: vec![ground],
                    grids: SceneGrids::default(),
                },
            },
        })
    }

    fn check(&self, cx: &mut RowCheck) {
        cx.text("/name", &self.name, 1, 60);
        self.blueprint.map.check("/blueprint/map", MIN_SIDE, cx);
        if !self.blueprint.variables.is_empty() {
            cx.error(
                "/blueprint/variables",
                "out of range",
                format!("{} variables", self.blueprint.variables.len()),
                "no variables: an area is never placed, so nothing fills them",
            );
        }
    }
}

impl NamedContent for AreaDef {
    fn shown_name(&self) -> &str {
        &self.name
    }
}

impl Bundle for AreaDef {
    const PARTS: &'static [PartSpec] = SCENE_PARTS;
    const WRITTEN: &'static [PartSpec] = SCENE_PARTS;

    fn read_parts(&mut self, parts: &mut Parts) {
        let blueprint = &mut self.blueprint;
        scene::read(
            &mut blueprint.scene,
            &blueprint.map,
            &blueprint.variables,
            parts,
        );
    }

    fn write_parts(&self, bundle: &str, writer: &mut FileWriter) -> io::Result<()> {
        scene::write(&self.blueprint.scene, &self.blueprint.map, bundle, writer)
    }
}

impl AreaDef {
    pub fn map(&self) -> &Tilemap {
        &self.blueprint.map
    }

    pub fn scene(&self) -> &Scene {
        &self.blueprint.scene
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tilemap {
    pub size: MapSize,
    pub tile: u32,
}

impl Tilemap {
    pub fn fresh(size: MapSize) -> Tilemap {
        Tilemap { size, tile: 16 }
    }

    pub(super) fn check(&self, pointer: &str, min_side: u32, cx: &mut RowCheck) {
        cx.range(
            format!("{pointer}/size/width"),
            self.size.width,
            min_side,
            MAX_SIDE,
            "tiles",
        );
        cx.range(
            format!("{pointer}/size/height"),
            self.size.height,
            min_side,
            MAX_SIDE,
            "tiles",
        );
        if self.size.width.saturating_mul(self.size.height) > MAX_CELLS {
            cx.error(
                format!("{pointer}/size"),
                "out of range",
                format!("{} cells", self.size.width * self.size.height),
                format!("at most {MAX_CELLS} cells"),
            );
        }
        cx.range(
            format!("{pointer}/tile"),
            self.tile,
            1,
            tileset::MAX_TILE_PX,
            "pixels per tile",
        );
    }
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapSize {
    pub width: u32,
    pub height: u32,
}

impl MapSize {
    pub fn grid(self) -> GridSize {
        GridSize::new(self.width, self.height)
    }

    pub fn tiles(self) -> Size<Meters> {
        Size::new(self.width as f32, self.height as f32)
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AreaModule {
    Object(ObjectDef),
    SafeZone(SafeZone),
    Soundscape(SoundscapeDef),
    Scenery(Scenery),
    Blueprint(Placement),
    Population(Population),
    Resident(Resident),
    Script(ScriptModule),
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, EnumString, IntoStaticStr, VariantArray,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum AreaModuleKind {
    Object,
    SafeZone,
    Soundscape,
    Scenery,
    Blueprint,
    Population,
    Resident,
    Script,
}

impl AreaModuleKind {
    pub fn tag(self) -> &'static str {
        self.into()
    }
}

impl AreaModule {
    pub fn kind(&self) -> AreaModuleKind {
        match self {
            AreaModule::Object(_) => AreaModuleKind::Object,
            AreaModule::SafeZone(_) => AreaModuleKind::SafeZone,
            AreaModule::Soundscape(_) => AreaModuleKind::Soundscape,
            AreaModule::Scenery(_) => AreaModuleKind::Scenery,
            AreaModule::Blueprint(_) => AreaModuleKind::Blueprint,
            AreaModule::Population(_) => AreaModuleKind::Population,
            AreaModule::Resident(_) => AreaModuleKind::Resident,
            AreaModule::Script(_) => AreaModuleKind::Script,
        }
    }

    fn check(&self, pointer: &str, cx: &mut RowCheck) {
        match self {
            AreaModule::Population(population) => cx.range(
                format!("{pointer}/count"),
                population.count,
                1,
                MAX_POPULATION,
                "npcs",
            ),
            AreaModule::Soundscape(soundscape) => soundscape.check(pointer, cx),
            AreaModule::Scenery(scenery) => scenery.check(pointer, cx),
            _ => {}
        }
    }
}

#[derive(JsonSchema, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SafeZone {
    #[schemars(with = "serial::RectRow")]
    #[serde(
        serialize_with = "serial::rect",
        deserialize_with = "serial::rect_from"
    )]
    pub rect: Rect<Meters>,
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoundscapeDef {
    pub shape: SoundscapeShape,
    pub emit: f32,
    pub channels: Vec<SoundChannel>,
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoundChannel {
    pub channel: u32,
    pub file: AudioRef,
    #[schemars(extend("default" = serial::one_f32()))]
    #[serde(
        default = "serial::one_f32",
        skip_serializing_if = "serial::is_one_f32"
    )]
    pub volume: f32,
    #[serde(default, skip_serializing_if = "serial::is_zero_f32")]
    pub pan: f32,
    #[serde(default, skip_serializing_if = "is_ambience")]
    pub category: AudioCategory,
}

fn is_ambience(category: &AudioCategory) -> bool {
    *category == AudioCategory::Ambience
}

impl SoundscapeDef {
    fn check(&self, pointer: &str, cx: &mut RowCheck) {
        cx.range(format!("{pointer}/emit"), self.emit, 0.0, 32.0, "m");
        cx.count(
            format!("{pointer}/channels"),
            self.channels.len(),
            1,
            8,
            "channels",
        );
        for (index, channel) in self.channels.iter().enumerate() {
            let at = format!("{pointer}/channels/{index}");
            cx.range(
                format!("{at}/channel"),
                channel.channel,
                1,
                16,
                "a channel number",
            );
            cx.range(format!("{at}/volume"), channel.volume, 0.0, 4.0, "volume");
            cx.range(format!("{at}/pan"), channel.pan, -1.0, 1.0, "pan");
            if self.channels[..index]
                .iter()
                .any(|other| other.channel == channel.channel)
            {
                cx.error(
                    format!("{at}/channel"),
                    "duplicate name",
                    format!("channel {} twice", channel.channel),
                    "each channel number once per soundscape",
                );
            }
        }
    }

    pub fn zone(&self) -> SoundscapeZone {
        SoundscapeZone {
            shape: self.shape,
            emit: Meters(self.emit),
            channels: self
                .channels
                .iter()
                .map(|channel| {
                    (
                        SoundscapeChannel(channel.channel),
                        SoundscapeLayer {
                            src: channel.file.clone(),
                            volume: channel.volume,
                            pan: channel.pan,
                            category: channel.category,
                        },
                    )
                })
                .collect::<BTreeMap<_, _>>(),
        }
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenery {
    #[schemars(with = "serial::PosRow")]
    #[serde(serialize_with = "serial::pos", deserialize_with = "serial::pos_from")]
    pub at: Pos<Meters>,
    pub tileset: TilesetId,
    pub tiles: Vec<Vec<Option<SceneryTile>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<f32>,
    #[serde(default, skip_serializing_if = "Flip::is_none")]
    pub flip: Flip,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<[i32; 2]>,
}

impl Scenery {
    fn check(&self, pointer: &str, cx: &mut RowCheck) {
        let width = self.tiles.first().map_or(0, Vec::len);
        if self.tiles.is_empty() || width == 0 {
            cx.error(
                format!("{pointer}/tiles"),
                "out of range",
                "no tiles",
                "at least one row of tiles",
            );
        }
        for (row, tiles) in self.tiles.iter().enumerate() {
            if tiles.len() != width {
                cx.error(
                    format!("{pointer}/tiles/{row}"),
                    "out of range",
                    format!("a row of {} tiles", tiles.len()),
                    format!("rows of {width} tiles, like the first"),
                );
            }
        }
        if let Some(sort) = self.sort {
            cx.range(
                format!("{pointer}/sort"),
                sort,
                0.0,
                self.tiles.len() as f32,
                "tiles from the top of the grid",
            );
        }
    }

    pub fn height(&self) -> usize {
        self.tiles.len()
    }

    pub fn width(&self) -> usize {
        self.tiles.first().map_or(0, Vec::len)
    }

    pub fn bounds(&self) -> Rect<Meters> {
        let size = Size::new(self.width() as f32, self.height() as f32);
        self.flip
            .place(Rect::new(Pos::new(0.0, 0.0), size), self.at)
    }

    pub fn sort_line(&self) -> Option<Meters> {
        Some(Meters(self.bounds().min().y + self.sort?))
    }

    /// The cell under the middle of the tile `[column, row]`, as the scenery is placed and flipped;
    /// a block names its cell this way.
    pub fn cell_under(&self, [column, row]: [i32; 2]) -> CellPos {
        let middle = Pos::new(column as f32 + 0.5, row as f32 + 0.5);
        (self.at + self.flip.apply(middle).to_vector()).cell()
    }

    /// Every tile of its grid as `[column, row]`, with the cell under it.
    pub fn cells(&self) -> impl Iterator<Item = ([i32; 2], CellPos)> + '_ {
        let (width, height) = (self.width() as i32, self.height() as i32);
        (0..height)
            .flat_map(move |row| (0..width).map(move |column| [column, row]))
            .map(|tile| (tile, self.cell_under(tile)))
    }
}

/// The depth each of `scenery` is drawn at in an area `height` tiles tall whose ground ends at
/// `base`: a sorted one at its sort line, among whoever stands there, and a flat one under them
/// all, later ones over earlier. The renderer orders equal depths as it likes, and that changes when
/// unrelated scenery moves, so scenery sharing a sort line is spread just below it in scene order,
/// staying under anyone standing on the line.
pub fn scenery_depths(scenery: &[Scenery], height: f32, base: f32) -> Vec<f32> {
    let flat_count = scenery.iter().filter(|s| s.sort_line().is_none()).count();
    let mut flat_index = 0;
    let mut depths: Vec<f32> = scenery
        .iter()
        .map(|s| match s.sort_line() {
            Some(line) => dynamic_z(height, base, line),
            None => {
                flat_index += 1;
                base - 1.0 + flat_index as f32 / (flat_count as f32 + 1.0)
            }
        })
        .collect();
    let mut lowest: HashMap<u32, f32> = HashMap::new();
    for depth in depths.iter_mut().rev() {
        let line = depth.to_bits();
        let placed = lowest.get(&line).map_or(*depth, |below| below.next_down());
        lowest.insert(line, placed);
        *depth = placed;
    }
    depths
}

#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "SceneryTileRow", into = "SceneryTileRow")]
pub struct SceneryTile {
    pub tile: u32,
    pub flip: Flip,
}

#[derive(JsonSchema, Serialize, Deserialize)]
#[serde(untagged)]
enum SceneryTileRow {
    Tile(u32),
    Flipped {
        tile: u32,
        #[serde(default)]
        flip: Flip,
    },
}

impl From<SceneryTileRow> for SceneryTile {
    fn from(row: SceneryTileRow) -> SceneryTile {
        match row {
            SceneryTileRow::Tile(tile) => SceneryTile {
                tile,
                flip: Flip::None,
            },
            SceneryTileRow::Flipped { tile, flip } => SceneryTile { tile, flip },
        }
    }
}

impl From<SceneryTile> for SceneryTileRow {
    fn from(tile: SceneryTile) -> SceneryTileRow {
        if tile.flip.is_none() {
            SceneryTileRow::Tile(tile.tile)
        } else {
            SceneryTileRow::Flipped {
                tile: tile.tile,
                flip: tile.flip,
            }
        }
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Population {
    pub npc: NpcId,
    #[schemars(extend("x-count" = true))]
    pub count: u32,
    #[schemars(with = "Option<serial::RectRow>")]
    #[serde(
        default,
        serialize_with = "serial::opt_rect",
        deserialize_with = "serial::opt_rect_from",
        skip_serializing_if = "Option::is_none"
    )]
    pub roams: Option<Rect<Meters>>,
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resident {
    pub npc: NpcId,
    #[schemars(with = "serial::PosRow")]
    #[serde(serialize_with = "serial::pos", deserialize_with = "serial::pos_from")]
    pub at: Pos<Meters>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shown: Option<Condition>,
}

pub struct PlacedModules(pub Vec<Vec<Flattened>>);

pub struct PlacedBlueprints(pub Vec<Vec<PlacedBlueprint>>);

pub fn placed(content: &Content, area: AreaId) -> &[Flattened] {
    content
        .built::<PlacedModules>()
        .0
        .get(area.index())
        .map_or(&[], Vec::as_slice)
}

pub fn placements(content: &Content, area: AreaId) -> &[PlacedBlueprint] {
    content
        .built::<PlacedBlueprints>()
        .0
        .get(area.index())
        .map_or(&[], Vec::as_slice)
}

pub fn modules(content: &Content, area: AreaId) -> impl Iterator<Item = &AreaModule> {
    placed(content, area).iter().map(|placed| &placed.module)
}

/// Where an npc lives: the first area that places it as a resident, at its spot there.
pub fn residence(content: &Content, npc: NpcId) -> Option<script::Place> {
    content.ids::<AreaDef>().find_map(|area| {
        modules(content, area).find_map(|module| match module {
            AreaModule::Resident(resident) if resident.npc == npc => Some(script::Place {
                area,
                at: resident.at,
            }),
            _ => None,
        })
    })
}

pub fn objects(modules: &[Flattened]) -> impl Iterator<Item = (u32, &ObjectDef)> {
    modules
        .iter()
        .enumerate()
        .filter_map(|(index, placed)| match &placed.module {
            AreaModule::Object(object) => Some((index as u32, object)),
            _ => None,
        })
}

pub fn npcs(modules: &[Flattened]) -> impl Iterator<Item = NpcId> + '_ {
    modules.iter().filter_map(|placed| match &placed.module {
        AreaModule::Resident(resident) => Some(resident.npc),
        AreaModule::Population(population) => Some(population.npc),
        _ => None,
    })
}

pub fn residents(modules: &[Flattened]) -> impl Iterator<Item = NpcId> + '_ {
    modules.iter().filter_map(|placed| match &placed.module {
        AreaModule::Resident(resident) => Some(resident.npc),
        _ => None,
    })
}

pub fn warp_objects(content: &Content, area: AreaId) -> Vec<(u32, &ObjectDef)> {
    if !content.has_built::<RiftScripts>() {
        return Vec::new();
    }
    let scripts = content.built::<RiftScripts>();
    objects(placed(content, area))
        .filter(|(index, _)| {
            scripts
                .may_do(OwnerRow::of_object(area, *index))
                .reaches
                .iter()
                .any(|reach| matches!(reach, Reach::Warp(_)))
        })
        .collect()
}

pub fn walkable(world: &World, tile: Pos<Meters>) -> bool {
    crate::systems::player::session::my_character(world)
        .map(|me| me.id())
        .and_then(|entity| of(world, entity))
        .is_some_and(|area| area.grid.walkable(tile))
}

pub struct Area {
    pub size: Size<Meters>,
    pub tile: u32,
    pub grid: movement::Grid,
    pub wild_grid: movement::Grid,
    pub airspace: movement::Grid,
    pub wild_airspace: movement::Grid,
    pub safe_zones: Vec<Rect<Meters>>,
    pub safe_cells: Vec<bool>,
    pub footsteps: Vec<Option<SfxId>>,
    pub soundscape: Vec<SoundscapeZone>,
    pub grids: Arc<AreaGrids>,
    pub scenery: Vec<Scenery>,
}

impl Area {
    pub fn safe(&self, at: Pos<Meters>) -> bool {
        at.cell()
            .index(self.size.grid())
            .is_some_and(|index| self.safe_cells[index])
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

    pub fn sort_base(&self) -> f32 {
        self.grids.ground.len() as f32
    }

    pub fn footstep_at(&self, c: CellPos) -> Option<&SfxId> {
        let i = c.index(self.size.grid())?;
        self.footsteps[i].as_ref()
    }

    pub fn fallback_spawn(&self) -> Pos<Meters> {
        self.grid
            .nearest_walkable(self.center())
            .unwrap_or_else(|| self.center())
    }

    pub fn center(&self) -> Pos<Meters> {
        Pos::new(
            (self.size.width / 2.0).floor(),
            (self.size.height / 2.0).floor(),
        )
    }
}

pub struct Areas(Vec<Option<Arc<Area>>>);

pub struct AreaGridSet(Vec<Option<Arc<AreaGrids>>>);

pub fn load(content: &Content, area: AreaId) -> &Area {
    shared(content, area)
}

/// The area as `load` gives it, to keep beside the content it came from.
pub fn shared(content: &Content, area: AreaId) -> &Arc<Area> {
    content
        .built::<Areas>()
        .0
        .get(area.index())
        .and_then(Option::as_ref)
        .unwrap_or_else(|| panic!("area {} failed to load", area.path(content)))
}

/// A bundle the editor pictures: an area, or a blueprint on its own canvas.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pictured {
    Area(AreaId),
    Blueprint(blueprint::BlueprintId),
}

/// What a bundle's scene makes, as the editor shows it.
pub struct Picture {
    /// The area the bundle makes with every node in it, as the game builds it.
    pub area: Arc<Area>,
    pub placed: Vec<Flattened>,
    pub placements: Vec<PlacedBlueprint>,
    /// The ground drawn: the bundle's own cells, stamped by the placements kept.
    pub ground: Arc<AreaGrids>,
    /// The scenery drawn: what the nodes kept bring.
    pub scenery: Vec<Scenery>,
}

/// Which module nodes a picture draws, each named by its path as `blueprint::flatten_kept` names
/// it.
#[derive(Clone, Copy)]
pub enum Kept<'a> {
    All,
    Only(&'a dyn Fn(&[&str]) -> bool),
}

/// The picture of `pictured`, drawing the module nodes `kept`. A blueprint is pictured on its own
/// canvas with its holes filled by their defaults.
pub fn picture(content: &Content, pictured: Pictured, kept: Kept) -> Option<Picture> {
    let own = content.built::<OwnGrids>();
    let own = match pictured {
        Pictured::Area(id) => own.areas.get(id.index())?.as_ref()?,
        Pictured::Blueprint(id) => own.blueprints.get(id.index())?.as_ref()?,
    };
    let mut errors = Vec::new();
    let flatten = |keep: &dyn Fn(&[&str]) -> bool, errors: &mut Vec<_>| match pictured {
        Pictured::Area(id) => content
            .table::<AreaDef>()
            .get(id)
            .map(|def| blueprint::flatten_kept(content, &def.scene().nodes, keep, errors)),
        Pictured::Blueprint(id) => Some(blueprint::flatten_blueprint(content, id, keep, errors)),
    };
    let (area, placed, placements) = match pictured {
        Pictured::Area(id) => (
            shared(content, id).clone(),
            self::placed(content, id).to_vec(),
            self::placements(content, id).to_vec(),
        ),
        Pictured::Blueprint(id) => {
            let def = content.table::<BlueprintDef>().get(id)?;
            let all = flatten(&|_| true, &mut errors)?;
            let grids = Arc::new(stamp_placed(content, own, &all.placements, &mut errors));
            let area = Arc::new(build_area(content, &def.map, grids, &all.modules));
            (area, all.modules, all.placements)
        }
    };
    let (ground, scenery) = match kept {
        Kept::All => (area.grids.clone(), area.scenery.clone()),
        Kept::Only(keep) => {
            let kept = flatten(keep, &mut errors)?;
            let ground = Arc::new(stamp_placed(content, own, &kept.placements, &mut errors));
            (ground, sceneries(&kept.modules))
        }
    };
    Some(Picture {
        area,
        placed,
        placements,
        ground,
        scenery,
    })
}

pub fn of(world: &World, entity: Entity) -> Option<&Area> {
    of_in(world.resource::<Content>(), world, entity)
}

pub fn of_in<'c>(content: &'c Content, world: &World, entity: Entity) -> Option<&'c Area> {
    let area = world.get::<AreaTag>(entity)?.area;
    Some(load(content, area))
}

/// The grids every area and blueprint paints itself, before placements stamp theirs.
struct OwnGrids {
    areas: Vec<Option<AreaGrids>>,
    blueprints: Vec<Option<AreaGrids>>,
}

fn resolve_grids(loading: &mut Loading) {
    let content = loading.content();
    let sizes = content.built::<ImageSizes>();
    let mut errors = Vec::new();
    let mut own = OwnGrids {
        areas: content.ids::<AreaDef>().map(|_| None).collect(),
        blueprints: content.ids::<BlueprintDef>().map(|_| None).collect(),
    };
    for (id, def) in loading.rows::<AreaDef>(&content) {
        own.areas[id.index()] = own_grids(
            &content,
            sizes,
            &def.blueprint,
            id.path(&content),
            &mut errors,
        );
    }
    for (id, def) in loading.rows::<BlueprintDef>(&content) {
        own.blueprints[id.index()] =
            own_grids(&content, sizes, def, id.path(&content), &mut errors);
    }
    for (id, _) in loading.rows::<AreaDef>(&content) {
        if own.areas[id.index()].is_none() {
            loading.fail(id);
        }
    }
    for (id, _) in loading.rows::<BlueprintDef>(&content) {
        if own.blueprints[id.index()].is_none() {
            loading.fail(id);
        }
    }
    loading.extend_errors(errors);
    loading.built(own);
}

fn own_grids(
    content: &Content,
    sizes: &ImageSizes,
    def: &BlueprintDef,
    bundle: &str,
    errors: &mut Vec<LoadError>,
) -> Option<AreaGrids> {
    let scene = &def.scene;
    let palette = grid::resolve_palette(
        content,
        sizes,
        &def.map,
        &format!("{bundle}/{}", scene::PALETTE_FILE),
        &scene.grids.palette,
        errors,
    )?;
    Some(AreaGrids {
        size: def.map.size.grid(),
        palette,
        palette_keys: scene.grids.palette.clone(),
        ground: blueprint::layer_grids(scene, &def.map),
        walk: scene.grids.walk.clone(),
    })
}

/// The cells of `own` with every one of `placements` stamped over them.
fn stamp_placed(
    content: &Content,
    own: &AreaGrids,
    placements: &[PlacedBlueprint],
    errors: &mut Vec<blueprint::FlattenError>,
) -> AreaGrids {
    let grids = content.built::<OwnGrids>();
    let blueprints = content.table::<BlueprintDef>();
    let grids_of = |blueprint: blueprint::BlueprintId| {
        Some((
            blueprints.get(blueprint)?,
            grids.blueprints.get(blueprint.index())?.as_ref()?,
        ))
    };
    blueprint::stamp(own, placements, grids_of, content, errors)
}

fn sceneries(modules: &[Flattened]) -> Vec<Scenery> {
    modules
        .iter()
        .filter_map(|placed| match &placed.module {
            AreaModule::Scenery(scenery) => Some(scenery.clone()),
            _ => None,
        })
        .collect()
}

fn flatten_all(loading: &mut Loading) {
    let content = loading.content();
    let own = content.built::<OwnGrids>();
    let mut placed: Vec<Vec<Flattened>> = content.ids::<AreaDef>().map(|_| Vec::new()).collect();
    let mut placements: Vec<Vec<PlacedBlueprint>> =
        content.ids::<AreaDef>().map(|_| Vec::new()).collect();
    let mut stamped: Vec<Option<Arc<AreaGrids>>> = content.ids::<AreaDef>().map(|_| None).collect();
    let loaded: BTreeSet<blueprint::BlueprintId> = loading
        .rows::<BlueprintDef>(&content)
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    let blueprints = content.table::<BlueprintDef>();
    for (id, def) in loading.rows::<AreaDef>(&content) {
        let mut errors = Vec::new();
        let flattening = blueprint::flatten(&content, &def.scene().nodes, &mut errors);
        let places_a_failed_blueprint = flattening
            .placements
            .iter()
            .any(|placed| !loaded.contains(&placed.blueprint));
        if let Some(area) = &own.areas[id.index()]
            && !places_a_failed_blueprint
        {
            let grids_of = |blueprint: blueprint::BlueprintId| {
                Some((
                    blueprints.get(blueprint)?,
                    own.blueprints[blueprint.index()].as_ref()?,
                ))
            };
            let grids = blueprint::stamp(
                area,
                &flattening.placements,
                grids_of,
                &content,
                &mut errors,
            );
            stamped[id.index()] = Some(Arc::new(grids));
        }
        if !errors.is_empty() || places_a_failed_blueprint {
            loading.fail(id);
        }
        let scene = scene_file(id.path(&content));
        for error in errors {
            match error.at {
                FlattenAt::Area(pointer) => loading.json_error(&scene, &pointer, error.problem),
                FlattenAt::Blueprint(blueprint, part, pointer) => {
                    loading.part_error(blueprint, part, &pointer, error.problem)
                }
            }
        }
        placed[id.index()] = flattening.modules;
        placements[id.index()] = flattening.placements;
    }
    loading.built(PlacedModules(placed));
    loading.built(PlacedBlueprints(placements));
    loading.built(AreaGridSet(stamped));
}

fn build_all(loading: &mut Loading) {
    let content = loading.content();
    let grids = content.built::<AreaGridSet>();
    let sizes = content.built::<ImageSizes>();
    let mut areas: Vec<Option<Arc<Area>>> = content.ids::<AreaDef>().map(|_| None).collect();
    let mut reported: BTreeSet<(String, String)> = BTreeSet::new();
    for (id, def) in loading.rows::<AreaDef>(&content) {
        let Some(grids) = grids.0[id.index()].clone() else {
            continue;
        };
        let modules = placed(&content, id);
        for module in modules {
            let (file, pointer) = module.site(&content, id);
            let mut check = RowCheck::default();
            module.module.check(&pointer, &mut check);
            for error in check.into_errors() {
                if reported.insert((file.clone(), error.pointer.clone())) {
                    loading.json_error(&file, &error.pointer, error.problem);
                }
            }
        }
        let map = def.map();
        let mut check = RowCheck::default();
        check_bounds(&content, map, modules, sizes, &mut check);
        let area = build_area(&content, map, grids, modules);
        check_placed(&content, id, &area, modules, &mut check);
        let scene = scene_file(id.path(&content));
        for error in check.into_errors() {
            loading.json_error(&scene, &error.pointer, error.problem);
        }
        areas[id.index()] = Some(Arc::new(area));
    }
    loading.built(Areas(areas));
}

/// The area a map, its stamped grids and its flattened modules make: where it walks, flies, is
/// safe and sounds.
fn build_area(
    content: &Content,
    map: &Tilemap,
    grids: Arc<AreaGrids>,
    modules: &[Flattened],
) -> Area {
    let safe_zones: Vec<Rect<Meters>> = modules
        .iter()
        .filter_map(|placed| match &placed.module {
            AreaModule::SafeZone(zone) => Some(zone.rect),
            _ => None,
        })
        .collect();
    let scenery = sceneries(modules);
    let walk = walk::grids(content, &grids, &scenery, &safe_zones);
    let size = map.size.grid();
    Area {
        size: map.size.tiles(),
        tile: map.tile,
        grid: movement::Grid::new(size, walk.walk),
        wild_grid: movement::Grid::new(size, walk.wild),
        airspace: movement::Grid::new(size, walk.airspace),
        wild_airspace: movement::Grid::new(size, walk.wild_airspace),
        safe_zones,
        safe_cells: walk.safe_cells,
        footsteps: walk.footsteps,
        soundscape: modules
            .iter()
            .filter_map(|placed| match &placed.module {
                AreaModule::Soundscape(soundscape) => Some(soundscape.zone()),
                _ => None,
            })
            .collect(),
        grids,
        scenery,
    }
}

/// Where in the area's scene a problem with `placed` is reported: at its own node and field, or at
/// the placement that brings it.
fn module_pointer(placed: &Flattened, field: &str) -> String {
    if placed.chain.is_empty() {
        format!("{}{field}", placed.origin)
    } else {
        placed.origin.clone()
    }
}

fn check_bounds(
    content: &Content,
    map: &Tilemap,
    modules: &[Flattened],
    sizes: &ImageSizes,
    check: &mut RowCheck,
) {
    let bounds = map.size.tiles().bounds();
    let outside = |at: Pos<Meters>| !bounds.contains(at);
    for placed in modules {
        let site = |field: &str| module_pointer(placed, field);
        let what = placed.describe(content);
        let out = |check: &mut RowCheck, at: Pos<Meters>| {
            check.error(
                site(""),
                "outside the map",
                format!("{what} lands outside the map at {}, {}", at.x, at.y),
                format!(
                    "everything inside the {} × {} map",
                    map.size.width, map.size.height
                ),
            );
        };
        match &placed.module {
            AreaModule::Object(object) => {
                let b = object.shape.bounds();
                if outside(b.min()) || outside(b.max() - Offset::new(0.001, 0.001)) {
                    out(check, object.shape.origin());
                }
            }
            AreaModule::Scenery(scenery) => {
                let tileset = scenery.tileset.get(content);
                if tileset.tile != map.tile {
                    check.error(
                        site("/tileset"),
                        "tile size mismatch",
                        format!("{} against {}", map.tile, tileset.tile),
                        format!("tilesets of {} px tiles, the area's", map.tile),
                    );
                }
                let count = sizes
                    .0
                    .get(tileset.image.as_str())
                    .map(|&(w, h)| tileset.grid(w, h).count());
                for (row, tiles) in scenery.tiles.iter().enumerate() {
                    for (column, tile) in tiles.iter().enumerate() {
                        let Some(tile) = tile else {
                            continue;
                        };
                        let square = walk::tile_square(scenery, column, row);
                        if outside(square.origin) {
                            out(check, square.origin);
                        }
                        if let Some(count) = count
                            && tile.tile >= count
                        {
                            check.error(
                                site(&format!("/tiles/{row}/{column}")),
                                "tile out of range",
                                format!("tile {} of {count}", tile.tile),
                                format!("0 to {}", count.saturating_sub(1)),
                            );
                        }
                    }
                }
            }
            AreaModule::Resident(resident) => {
                if outside(resident.at) {
                    out(check, resident.at);
                }
            }
            AreaModule::Population(Population {
                roams: Some(roams), ..
            }) if outside(roams.min()) => out(check, roams.min()),
            _ => {}
        }
    }
}

fn check_placed(
    content: &Content,
    id: AreaId,
    area: &Area,
    modules: &[Flattened],
    check: &mut RowCheck,
) {
    let warps: Vec<(u32, &ObjectDef, String)> = warp_objects(content, id)
        .into_iter()
        .map(|(index, object)| (index, object, object.display_name(index)))
        .collect();
    let in_warp = |at: Pos<Meters>| {
        warps
            .iter()
            .find(|(_, object, _)| object.shape.covers(at))
            .map(|(_, _, name)| name.clone())
    };
    let nearest_cell = |at: Pos<Meters>| {
        area.grid
            .nearest_walkable(at)
            .map(|cell| format!("{}, {}", cell.x, cell.y))
    };
    let mut names: Vec<(String, String)> = Vec::new();
    let mut seen_residents: Vec<NpcId> = Vec::new();
    for placed in modules {
        let site = |field: &str| module_pointer(placed, field);
        match &placed.module {
            AreaModule::Object(object) => {
                if let Some(name) = &object.name {
                    if let Some((_, first)) =
                        names.iter().find(|(n, _)| n.eq_ignore_ascii_case(name))
                    {
                        check.error(
                            site(""),
                            "duplicate name",
                            format!("{name}, from {} and {}", first, placed.describe(content)),
                            "object names unique in the area ignoring case",
                        );
                    } else {
                        names.push((name.clone(), placed.describe(content)));
                    }
                }
                if let ObjectShape::Point(at) = object.shape
                    && !area.grid.walkable(at)
                {
                    check.problem(
                        site("/shape"),
                        ContentProblem::new(
                            "not walkable",
                            format!("{}, {}", at.x, at.y),
                            "a point object on a walkable cell",
                        )
                        .near(nearest_cell(at)),
                    );
                }
            }
            AreaModule::Resident(resident) => {
                let key = resident.npc.path(content);
                if seen_residents.contains(&resident.npc) {
                    check.error(
                        site(""),
                        "resident twice",
                        format!("{key} in {}", id.path(content)),
                        "each npc resident once per area",
                    );
                }
                seen_residents.push(resident.npc);
                if !area.grid.walkable(resident.at) {
                    check.problem(
                        site("/at"),
                        ContentProblem::new(
                            "not walkable",
                            format!("{} {}, {}", key, resident.at.x, resident.at.y),
                            "a resident on a walkable cell",
                        )
                        .near(nearest_cell(resident.at)),
                    );
                } else if let Some(door) = in_warp(resident.at) {
                    check.error(
                        site("/at"),
                        "in a warp object",
                        format!(
                            "{key} {}, {} inside object {door}",
                            resident.at.x, resident.at.y
                        ),
                        "a spot outside every warp object",
                    );
                }
            }
            AreaModule::Population(population) => {
                let ground = area.wild_grid.nodes();
                let has_ground = match population.roams {
                    Some(bounds) => ground.iter().any(|&node| bounds.contains(node)),
                    None => !ground.is_empty(),
                };
                if !has_ground {
                    check.error(
                        site(""),
                        "no wild ground",
                        format!("{} in {}", population.npc.path(content), id.path(content)),
                        "a walkable cell outside every safe zone to spawn on",
                    );
                }
            }
            _ => {}
        }
    }
    for (index, (object_index, object, name)) in warps.iter().enumerate() {
        let pointer = module_pointer(&modules[*object_index as usize], "");
        let covered = object
            .shape
            .bounds()
            .tiles()
            .any(|cell| object.shape.covers_cell(cell) && area.grid.walkable(cell.center()));
        if !covered {
            check.error(
                pointer.clone(),
                "warp object has no ground",
                name.clone(),
                "a warp object covering at least one walkable cell",
            );
        }
        for (_, other, other_name) in &warps[index + 1..] {
            if object.shape.overlaps(&other.shape) {
                check.error(
                    pointer.clone(),
                    "warp objects overlap",
                    format!("{name} and {other_name}"),
                    "warp objects that never overlap",
                );
            }
        }
    }
}

pub fn check_place(content: &Content, place: script::Place) -> Result<(), ContentProblem> {
    let Some(area) = content
        .built::<Areas>()
        .0
        .get(place.area.index())
        .and_then(Option::as_ref)
    else {
        return Ok(());
    };
    let key = place.area.path(content);
    let warps = warp_objects(content, place.area);
    let problem = |rule: &'static str, found: String| {
        let nearest = area
            .grid
            .nodes()
            .iter()
            .filter(|cell| !warps.iter().any(|(_, object)| object.shape.covers(**cell)))
            .min_by(|a, b| a.distance(place.at).0.total_cmp(&b.distance(place.at).0))
            .map(|cell| format!("{}, {}", cell.x, cell.y));
        ContentProblem::new(
            rule,
            found,
            format!("a place on a walkable cell of {key}, outside every warp object"),
        )
        .near(nearest)
    };
    if !area.grid.walkable(place.at) {
        return Err(problem(
            "not walkable",
            format!("{key} {}, {}", place.at.x, place.at.y),
        ));
    }
    for &(index, object) in &warps {
        if object.shape.covers(place.at) {
            return Err(problem(
                "in a warp object",
                format!(
                    "{key} {}, {} inside object {}",
                    place.at.x,
                    place.at.y,
                    object.display_name(index)
                ),
            ));
        }
    }
    Ok(())
}
