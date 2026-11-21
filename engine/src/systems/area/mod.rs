pub mod blueprint;
pub mod fog;
pub mod heavens;
pub mod lighting;
pub mod object;
pub mod prop;
pub mod prop_material;
pub mod query;
pub mod rain;
pub mod render;
pub mod scene;
pub mod scenery;
pub mod script;
pub mod shown_weather;
pub mod sky;
pub mod spawnable;
pub mod stages;
pub mod terrain;
pub mod transition;
pub mod vfx;
pub mod walk;
pub mod warp;
pub mod weather;
pub mod world_map;

pub use blueprint::{BlueprintDef, Flattened, PlacedBlueprint, Placement};
pub use fog::{FogBank, LaidFog, Shafts};

pub use lighting::{Lighting, PointLightDef};
pub use object::{AreaObject, ObjectDef, ObjectShape};
pub use prop::{Footprint, PropDef, PropId, PropPlacement, Scatter};
pub use scene::SceneNode;
pub use script::{
    AREA_RUNNER, BLUEPRINT_RUNNER, OBJECT_RUNNER, PLACED_RUNNERS, Place, Started, Warps,
};
pub use terrain::{SurfaceDef, SurfaceId, TerrainDef, TerrainId, Walk};
pub use walk::{GroundHold, HeldMesh, Survey, SurveyAnswer, SurveyIsland, SurveyQuestion};
pub use weather::{
    Climate, DayCycle, Lightning, MAX_RAIN_SHELLS, NextSpell, RainSound, Rainfall, Spell,
    SpellCourse, SpellLength, TimeOfDay, TimeOfDayName, Weather, Wind,
};

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

use crate::core::audio::mix::AudioCategory;
use crate::core::audio::playback::SfxId;
use crate::core::audio::soundscape::{
    SoundscapeChannel, SoundscapeLayer, SoundscapeShape, SoundscapeZone,
};
use crate::core::content::ErrorPlace;
use crate::core::content::{AssetKind, AudioRef};
use crate::core::content::{
    Bundle, Content, ContentFile, ContentId, ContentProblem, ContentRegistry, FileShape,
    FileWriter, LoadStep, Loading, NamedContent, Nearby, PartSpec, Parts, RowCheck, fields_file,
    serial,
};
use crate::core::ground::{
    GroundGrid, GroundSize, GroundValue, HeightRange, Heightfield, MAX_TERRAINS, MIN_SIDE,
    PaintMap, SAMPLES_PER_METER, ShareMap, TILE_SIDE, TileAt, WaterMap, WaterSurface,
};
use crate::core::math::{Offset, Pos, Rect, Size};
use crate::core::script::{Expr, OwnerRow, ScriptModule, Scripts};
use crate::core::spatial::SpatialIndex;
use crate::core::units::{Degrees, GridDims, MeterSize, Meters, Seconds};
use crate::systems::GameSettings;
use crate::systems::actor::Pose;
use crate::systems::area::sky::SkyCondition;
use crate::systems::area::sky::SkyTable;
use crate::systems::area_instance::AreaInstanceRules;
use crate::systems::mount::MountKind;
use crate::systems::movement::{Bodies, NavMesh};
use crate::systems::npc::NpcId;
use crate::systems::spawner::{SpawnSurface, Spawner};
use crate::systems::water::{MAX_WATERS, WaterDef, WaterId};
use blueprint::{FlattenAt, FlattenError};
use lighting::MAX_LIGHTS;
use prop::MAX_PROPS;
use scene::{SCENE, scene_file};

pub type AreaId = ContentId<AreaDef>;

pub fn register(app: &mut App) {
    use bevy_replication::AppReplicationExt;
    app.replicate_once::<AreaTag>();
    object::register(app);
    vfx::register(app);
}

pub fn content(registry: &mut ContentRegistry) {
    registry
        .bundle::<PropDef>()
        .file::<prop_material::PropMaterialDef>()
        .file::<TerrainDef>()
        .named::<TerrainDef>()
        .file::<SurfaceDef>()
        .named::<SurfaceDef>()
        .bundle::<BlueprintDef>()
        .bundle::<AreaDef>()
        .named::<AreaDef>()
        .file::<world_map::WorldMapDef>()
        .file::<spawnable::SpawnableDef>()
        .named::<spawnable::SpawnableDef>()
        .modules::<spawnable::SpawnableDef>()
        .json_part(scene::json_part(AreaDef::TABLE))
        .json_part(scene::json_part(BlueprintDef::TABLE))
        .hook(LoadStep::Files, prop::check_fitted)
        .hook(LoadStep::Across, terrain::check_all)
        .hook(LoadStep::Across, vfx::check_sites)
        .hook(LoadStep::Across, terrain::check_pictures)
        .hook(LoadStep::Across, prop_material::check_all)
        .hook(LoadStep::Across, lighting::check_area_shaders)
        .hook(LoadStep::Expand, flatten_all)
        .hook(LoadStep::Assemble, build_all)
        .hook(LoadStep::Across, world_map::check_all)
        .hook(LoadStep::Settings, fog::check_all);
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

/// Ground can lean no further than upright.
const UPRIGHT: Degrees = Degrees(90.0);
/// Metres a pictured water's shore rises for each metre back from the waterline.
const SHORE_RISE: f32 = 0.15;

/// The ground's heights, paint and water, a tile of each to a file named by the tile's place, and
/// an overview of each over the whole ground, which answers for the tiles not loaded.
pub const HEIGHT: PartSpec = PartSpec::Keyed {
    folder: "height",
    exts: AssetKind::Image.extensions(),
};
pub const PAINT: PartSpec = PartSpec::Keyed {
    folder: "paint",
    exts: AssetKind::Image.extensions(),
};
pub const HEIGHT_OVERVIEW: PartSpec = PartSpec::File {
    name: "height_overview",
    exts: AssetKind::Image.extensions(),
};
pub const PAINT_OVERVIEW: PartSpec = PartSpec::File {
    name: "paint_overview",
    exts: AssetKind::Image.extensions(),
};
pub const WATER: PartSpec = PartSpec::Keyed {
    folder: "water",
    exts: AssetKind::Image.extensions(),
};
pub const WATER_OVERVIEW: PartSpec = PartSpec::File {
    name: "water_overview",
    exts: AssetKind::Image.extensions(),
};
pub const WATERS: PartSpec = PartSpec::Keyed {
    folder: "waters",
    exts: AssetKind::Image.extensions(),
};
pub const WATERS_OVERVIEW: PartSpec = PartSpec::File {
    name: "waters_overview",
    exts: AssetKind::Image.extensions(),
};
/// The share of the downpour each stretch of the area gets, as coarse as its maker likes: where
/// the bundle has none, rain falls alike everywhere.
pub const RAIN: PartSpec = PartSpec::File {
    name: "rain",
    exts: AssetKind::Image.extensions(),
};

/// A place in the world: a stretch of ground painted with terrains, with water lying on it, the
/// light it stands in, its climate and how its rain falls over it, and a scene of modules on it.
/// Its scene, the tiles of its heights, paint and water, and its rain map are parts of its bundle.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AreaDef {
    pub name: String,
    pub size: GroundSize,
    /// The heights the ground's tiles span: changing it stretches the land with it.
    pub height_range: HeightRange,
    /// How steep a slope still walks.
    pub max_slope: Degrees,
    /// The terrains the paint names by their index here: the first is the ground where nothing is
    /// painted. The editor adds a terrain as it is first painted.
    pub terrains: Vec<TerrainId>,
    /// The waters the water map names by their index here. The editor adds a water as it is
    /// first laid.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub waters: Vec<WaterId>,
    pub lighting: Lighting,
    #[serde(default, skip_serializing_if = "Climate::is_still")]
    pub climate: Climate,
    /// The kinds of mount nobody rides here: one called here is refused, and a rider who warps in
    /// arrives on foot.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bars_mounts: Vec<MountKind>,
    /// With it, the area is never shared: it is only ever entered as an instance, an instance of it
    /// that is one party's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<AreaInstanceRules>,
    #[serde(skip)]
    pub scene: Vec<SceneNode>,
    #[serde(skip)]
    pub heights: Heightfield,
    #[serde(skip)]
    pub paint: PaintMap,
    #[serde(skip)]
    pub water: WaterSurface,
    #[serde(skip)]
    pub water_map: WaterMap,
    #[serde(skip)]
    pub rain: Option<ShareMap>,
}

impl ContentFile for AreaDef {
    const TABLE: &'static str = "area";
    const SHAPE: FileShape = FileShape::Bundle;

    fn fresh(near: &Nearby) -> Result<AreaDef, String> {
        let size = GroundSize {
            width: 16,
            height: 12,
        };
        let height_range = HeightRange {
            lowest: Meters(-8.0),
            highest: Meters(16.0),
        };
        Ok(AreaDef {
            name: near.name(),
            size,
            height_range,
            max_slope: Degrees(38.0),
            terrains: vec![near.file()?],
            waters: Vec::new(),
            lighting: Lighting::day(),
            climate: Climate::always(Weather::breezy()),
            bars_mounts: Vec::new(),
            instance: None,
            scene: vec![SceneNode::Module(scene::ModuleNode::new(AreaModule::Fog(
                FogBank::light_over(size.bounds()),
            )))],
            heights: Heightfield::flat(size, height_range),
            paint: PaintMap::plain(size),
            water: WaterSurface::dry(size, height_range),
            water_map: WaterMap::plain(size),
            rain: None,
        })
    }

    fn check(&self, cx: &mut RowCheck) {
        cx.text("/name", &self.name, 1, 60);
        self.size.check("/size", MIN_SIDE, cx);
        self.height_range.check("/height_range", cx);
        cx.range(
            "/max_slope",
            self.max_slope.0,
            0.0,
            UPRIGHT.0,
            "degrees of slope",
        );
        cx.range(
            "/terrains",
            self.terrains.len(),
            1,
            MAX_TERRAINS,
            "terrains",
        );
        cx.range("/waters", self.waters.len(), 0, MAX_WATERS, "waters");
        self.lighting.check("/lighting", cx);
        self.climate.check("/climate", cx);
        if let Some(rules) = &self.instance {
            rules.check("/instance", cx);
        }
    }
}

impl NamedContent for AreaDef {
    fn shown_name(&self) -> &str {
        &self.name
    }
}

impl Bundle for AreaDef {
    const PARTS: &'static [PartSpec] = &[
        SCENE,
        HEIGHT,
        PAINT,
        WATER,
        WATERS,
        HEIGHT_OVERVIEW,
        PAINT_OVERVIEW,
        WATER_OVERVIEW,
        WATERS_OVERVIEW,
        RAIN,
    ];
    const WRITTEN: &'static [PartSpec] = Self::PARTS;
    const STREAMED: &'static [PartSpec] = &[HEIGHT, PAINT, WATER, WATERS];

    fn read_parts(&mut self, parts: &mut Parts) {
        scene::read(&mut self.scene, &BTreeMap::new(), parts);
        self.heights = Heightfield::flat(self.size, self.height_range);
        self.paint = PaintMap::plain(self.size);
        self.water = WaterSurface::dry(self.size, self.height_range);
        self.water_map = WaterMap::plain(self.size);
        read_ground(
            parts,
            GroundParts {
                tiles: HEIGHT,
                overview: HEIGHT_OVERVIEW,
            },
            &mut self.heights,
        );
        read_ground(
            parts,
            GroundParts {
                tiles: PAINT,
                overview: PAINT_OVERVIEW,
            },
            &mut self.paint,
        );
        read_ground(
            parts,
            GroundParts {
                tiles: WATER,
                overview: WATER_OVERVIEW,
            },
            &mut self.water,
        );
        read_ground(
            parts,
            GroundParts {
                tiles: WATERS,
                overview: WATERS_OVERVIEW,
            },
            &mut self.water_map,
        );
        self.rain = read_share_map(parts, RAIN);
    }

    fn write_parts(&self, bundle: &str, writer: &mut FileWriter) -> io::Result<()> {
        scene::write(&self.scene, bundle, writer)?;
        let part = |spec: PartSpec, key: &str| format!("{bundle}/{}", spec.named(key, "png"));
        for at in self.size.each_tile() {
            put(writer, part(HEIGHT, &at.key()), || {
                self.heights.encode_tile(at)
            })?;
            put(writer, part(PAINT, &at.key()), || {
                self.paint.encode_tile(at)
            })?;
            put(writer, part(WATER, &at.key()), || {
                self.water.encode_tile(at)
            })?;
            put(writer, part(WATERS, &at.key()), || {
                self.water_map.encode_tile(at)
            })?;
        }
        put(writer, part(HEIGHT_OVERVIEW, ""), || {
            self.heights.encode_overview()
        })?;
        put(writer, part(PAINT_OVERVIEW, ""), || {
            self.paint.encode_overview()
        })?;
        put(writer, part(WATER_OVERVIEW, ""), || {
            self.water.encode_overview()
        })?;
        put(writer, part(WATERS_OVERVIEW, ""), || {
            self.water_map.encode_overview()
        })?;
        put(writer, part(RAIN, ""), || {
            self.rain.as_ref().map(ShareMap::encode)
        })?;
        Ok(())
    }
}

impl AreaDef {
    /// The files of the area at `bundle` its ground keeps that differ from `before`'s: each tile
    /// of its heights, paint, water or water map whose values changed, and the overview of each
    /// that did.
    pub fn ground_files_changed(&self, before: &AreaDef, bundle: &str) -> BTreeSet<String> {
        let part = |spec: PartSpec, key: &str| format!("{bundle}/{}", spec.named(key, "png"));
        let resized = self.size != before.size;
        let mut files = BTreeSet::new();
        let mut grounds = [
            (HEIGHT, HEIGHT_OVERVIEW, false),
            (PAINT, PAINT_OVERVIEW, false),
            (WATER, WATER_OVERVIEW, false),
            (WATERS, WATERS_OVERVIEW, false),
        ];
        for at in self.size.each_tile() {
            let changed = [
                self.heights.version(at) != before.heights.version(at),
                self.paint.version(at) != before.paint.version(at),
                self.water.version(at) != before.water.version(at),
                self.water_map.version(at) != before.water_map.version(at),
            ];
            for ((tiles, _, any), changed) in grounds.iter_mut().zip(changed) {
                if resized || changed {
                    files.insert(part(*tiles, &at.key()));
                    *any = true;
                }
            }
        }
        for (_, overview, any) in grounds {
            if any {
                files.insert(part(overview, ""));
            }
        }
        files
    }

    /// The area over ground of `size`, its heights, paint and water kept from the top-left corner.
    pub fn resized(&self, size: GroundSize) -> AreaDef {
        let mut area = self.clone();
        area.size = size;
        area.heights = self.heights.resized(size);
        area.paint = self.paint.resized(size);
        area.water = self.water.resized(size);
        area.water_map = self.water_map.resized(size);
        area.rain = self
            .rain
            .as_ref()
            .map(|rain| rain.resized(self.size.meters(), size.meters()));
        area
    }

    pub fn size(&self) -> Size<Meters> {
        self.size.meters()
    }

    /// The index the paint names `terrain` by, listing it when it is new.
    pub fn terrain_index(&mut self, terrain: TerrainId) -> Option<u8> {
        if let Some(index) = self.terrains.iter().position(|known| *known == terrain) {
            return u8::try_from(index).ok();
        }
        if self.terrains.len() >= MAX_TERRAINS {
            return None;
        }
        self.terrains.push(terrain);
        u8::try_from(self.terrains.len() - 1).ok()
    }

    /// The index the water map names `water` by, listing it when it is new.
    pub fn water_index(&mut self, water: WaterId) -> Option<u8> {
        if let Some(index) = self.waters.iter().position(|known| *known == water) {
            return u8::try_from(index).ok();
        }
        if self.waters.len() >= MAX_WATERS {
            return None;
        }
        self.waters.push(water);
        u8::try_from(self.waters.len() - 1).ok()
    }

    fn ground(&self) -> Ground {
        Ground {
            size: self.size,
            max_slope: self.max_slope,
            terrains: self.terrains.clone(),
            waters: self.waters.clone(),
            heights: self.heights.clone(),
            paint: self.paint.clone(),
            water: self.water.clone(),
            water_map: self.water_map.clone(),
            rain: self.rain.clone(),
            lighting: self.lighting.clone(),
            climate: self.climate.clone(),
        }
    }
}

/// Reads a map of shares over the ground from the bundle's file for `spec`, refusing at the file
/// one that doesn't decode; none where the bundle has no such file.
fn read_share_map(parts: &mut Parts, spec: PartSpec) -> Option<ShareMap> {
    let file = parts.file(spec)?;
    let bytes = parts.bytes(file)?;
    match ShareMap::decode(&bytes) {
        Ok(map) => Some(map),
        Err(problem) => {
            parts.error(file, ErrorPlace::WholeFile, problem);
            None
        }
    }
}

/// The parts one grid of the ground is kept in: its tiles and their overview.
#[derive(Clone, Copy)]
struct GroundParts {
    tiles: PartSpec,
    overview: PartSpec,
}

/// Reads one grid of the ground from the bundle into `ground`: every tile, refusing at its file
/// one that isn't a tile of a ground of `size` or doesn't decode, and the overview, which a ground
/// with any tile must keep. A load to play leaves the tiles stored, to read as they are first
/// needed; a check decodes them all, on every core, and holds the overview to what they say,
/// unless the folder vouches for every tile and the overview as they stand. A tile the bundle
/// leaves out stays as it is.
fn read_ground<T: GroundValue>(parts: &mut Parts, specs: GroundParts, ground: &mut GroundGrid<T>) {
    let (across, down) = ground.size().tiles();
    let tiles = parts.keyed(specs.tiles);
    let any_tile = !tiles.is_empty();
    let mut files = HashMap::new();
    for (key, file) in tiles {
        let Some(at) = TileAt::from_key(key).filter(|at| at.column < across && at.row < down)
        else {
            parts.error(
                file,
                ErrorPlace::WholeFile,
                ContentProblem::new(
                    "not a tile of the ground",
                    key,
                    format!(
                        "<column>_<row>, from 0_0 to {}_{} over this ground",
                        across - 1,
                        down - 1
                    ),
                ),
            );
            continue;
        };
        ground.store(at, parts.stored(file));
        files.insert(at, file);
    }
    let overview = if any_tile {
        parts.required(specs.overview)
    } else {
        parts.file(specs.overview)
    };
    let check = parts.thorough()
        && files
            .values()
            .copied()
            .chain(overview)
            .any(|file| !parts.vouched(file));
    if check {
        for (at, problem) in ground.load_all() {
            parts.error(files[&at], ErrorPlace::WholeFile, problem);
        }
    }
    let Some(file) = overview else {
        return;
    };
    let Some(bytes) = parts.bytes(file) else {
        return;
    };
    if let Err(problem) = ground.decode_overview(&bytes) {
        parts.error(file, ErrorPlace::WholeFile, problem);
        return;
    }
    if check && !ground.overview_is_current() {
        parts.error(
            file,
            ErrorPlace::WholeFile,
            ContentProblem::new(
                "overview out of date",
                "an overview unlike the tiles",
                "the overview the tiles make: write the area again",
            ),
        );
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AreaModule {
    Object(ObjectDef),
    SafeZone(SafeZone),
    Soundscape(SoundscapeDef),
    Prop(PropPlacement),
    Scatter(Scatter),
    Light(PointLightDef),
    Fog(FogBank),
    Blueprint(Placement),
    Spawner(Spawner),
    Resident(Resident),
    Script(ScriptModule),
}

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    EnumString,
    IntoStaticStr,
    VariantArray,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum AreaModuleKind {
    Object,
    SafeZone,
    Soundscape,
    Prop,
    Scatter,
    Light,
    Fog,
    Blueprint,
    Spawner,
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
            AreaModule::Prop(_) => AreaModuleKind::Prop,
            AreaModule::Scatter(_) => AreaModuleKind::Scatter,
            AreaModule::Light(_) => AreaModuleKind::Light,
            AreaModule::Fog(_) => AreaModuleKind::Fog,
            AreaModule::Blueprint(_) => AreaModuleKind::Blueprint,
            AreaModule::Spawner(_) => AreaModuleKind::Spawner,
            AreaModule::Resident(_) => AreaModuleKind::Resident,
            AreaModule::Script(_) => AreaModuleKind::Script,
        }
    }

    /// Moves what places the module on the ground by `dx`, `dy`, snapped to sixteenths of a metre.
    pub fn shift(&mut self, dx: f32, dy: f32) {
        let moved = |at: Pos<Meters>| Pos::new(sixteenth(at.x + dx), sixteenth(at.y + dy));
        let moved_rect = |rect: Rect<Meters>| Rect::new(moved(rect.origin), rect.size);
        match self {
            AreaModule::Object(object) => {
                object.shape = match object.shape {
                    ObjectShape::Point(at) => ObjectShape::Point(moved(at)),
                    ObjectShape::Rect(rect) => ObjectShape::Rect(moved_rect(rect)),
                };
            }
            AreaModule::Soundscape(soundscape) => {
                soundscape.shape = match soundscape.shape {
                    SoundscapeShape::Rect(rect) => SoundscapeShape::Rect(moved_rect(rect)),
                    SoundscapeShape::Ellipse(rect) => SoundscapeShape::Ellipse(moved_rect(rect)),
                };
            }
            AreaModule::SafeZone(zone) => zone.rect = moved_rect(zone.rect),
            AreaModule::Fog(bank) => bank.rect = moved_rect(bank.rect),
            AreaModule::Prop(prop) => prop.at = moved(prop.at),
            AreaModule::Scatter(scatter) => scatter.region = moved_rect(scatter.region),
            AreaModule::Light(light) => light.at = moved(light.at),
            AreaModule::Blueprint(placement) => placement.at = moved(placement.at),
            AreaModule::Resident(resident) => resident.at = moved(resident.at),
            AreaModule::Spawner(spawner) => spawner.within = spawner.within.map(moved_rect),
            AreaModule::Script(_) => {}
        }
    }

    /// Every condition of the sky the module follows, with where in the module it is written.
    pub fn whens(&self) -> Vec<(String, &SkyCondition)> {
        match self {
            AreaModule::Soundscape(soundscape) => soundscape
                .channels
                .iter()
                .enumerate()
                .filter_map(|(index, channel)| {
                    Some((format!("/channels/{index}/when"), channel.when.as_ref()?))
                })
                .collect(),
            AreaModule::Light(light) => light
                .when
                .iter()
                .map(|when| ("/when".to_owned(), when))
                .collect(),
            AreaModule::Fog(bank) => bank
                .when
                .iter()
                .map(|when| ("/when".to_owned(), when))
                .collect(),
            AreaModule::Spawner(spawner) => spawner
                .populations
                .iter()
                .enumerate()
                .flat_map(|(population, held)| {
                    held.spawns
                        .iter()
                        .enumerate()
                        .map(move |(index, share)| (population, index, share))
                })
                .filter_map(|(population, index, share)| {
                    Some((
                        format!("/populations/{population}/spawns/{index}/when"),
                        share.when.as_ref()?,
                    ))
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Holds the module, at `pointer` of its scene file, to the rules it keeps alone.
    pub fn check(&self, pointer: &str, cx: &mut RowCheck) {
        match self {
            AreaModule::Spawner(spawner) => spawner.check(pointer, cx),
            AreaModule::Soundscape(soundscape) => soundscape.check(pointer, cx),
            AreaModule::Prop(prop) => prop.check(pointer, cx),
            AreaModule::Scatter(scatter) => scatter.check(pointer, cx),
            AreaModule::Light(light) => light.check(pointer, cx),
            AreaModule::Fog(bank) => bank.check(pointer, cx),
            AreaModule::Resident(resident) => {
                cx.range(
                    format!("{pointer}/facing"),
                    resident.facing.0,
                    0.0,
                    360.0,
                    "degrees",
                );
                if let Some(respawn) = resident.respawn {
                    cx.at_least(format!("{pointer}/respawn"), respawn.0, 0.0, "seconds");
                }
            }
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
    /// How far from its shape it is heard, fading out as it goes.
    pub reach: Meters,
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
    /// How much of the sound is there under the sky, taking that share of its channel's power;
    /// all of it without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<SkyCondition>,
}

fn is_ambience(category: &AudioCategory) -> bool {
    *category == AudioCategory::Ambience
}

impl SoundscapeDef {
    fn check(&self, pointer: &str, cx: &mut RowCheck) {
        cx.range(format!("{pointer}/reach"), self.reach.0, 0.5, 64.0, "m");
        cx.count(
            format!("{pointer}/channels"),
            self.channels.len(),
            1,
            16,
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
                .any(|other| other.channel == channel.channel && other.file == channel.file)
            {
                cx.error(
                    format!("{at}/file"),
                    "duplicate name",
                    format!(
                        "{} twice on channel {}",
                        channel.file.as_str(),
                        channel.channel
                    ),
                    "each sound once per channel of a soundscape",
                );
            }
        }
    }

    /// The zone it sounds in, each of its sounds as much there as `share` says of its `when`.
    pub fn zone(&self, share: impl Fn(Option<&SkyCondition>) -> f32) -> SoundscapeZone {
        SoundscapeZone {
            shape: self.shape,
            reach: self.reach,
            layers: self
                .channels
                .iter()
                .map(|channel| SoundscapeLayer {
                    channel: SoundscapeChannel(channel.channel),
                    share: share(channel.when.as_ref()),
                    src: channel.file.clone(),
                    volume: channel.volume,
                    pan: channel.pan,
                    category: channel.category,
                })
                .collect(),
        }
    }
}

/// An npc living at a spot of the area, facing a way and holding a pose there.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resident {
    pub npc: NpcId,
    #[schemars(with = "serial::PosRow")]
    #[serde(serialize_with = "serial::pos", deserialize_with = "serial::pos_from")]
    pub at: Pos<Meters>,
    #[serde(default, skip_serializing_if = "Degrees::is_zero")]
    pub facing: Degrees,
    #[serde(default, skip_serializing_if = "is_idle")]
    pub pose: Pose,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shown: Option<Expr>,
    /// How long after its body goes it stands at its spot again. Without one it stays gone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub respawn: Option<Seconds>,
}

fn is_idle(pose: &Pose) -> bool {
    *pose == Pose::Idle
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
pub fn residence(content: &Content, npc: NpcId) -> Option<Place> {
    content.ids::<AreaDef>().find_map(|area| {
        modules(content, area).find_map(|module| match module {
            AreaModule::Resident(resident) if resident.npc == npc => Some(Place {
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
    modules
        .iter()
        .flat_map(|placed| -> Box<dyn Iterator<Item = NpcId> + '_> {
            match &placed.module {
                AreaModule::Resident(resident) => Box::new(std::iter::once(resident.npc)),
                AreaModule::Spawner(spawner) => Box::new(spawner.npcs()),
                _ => Box::new(std::iter::empty()),
            }
        })
}

pub fn residents(modules: &[Flattened]) -> impl Iterator<Item = NpcId> + '_ {
    modules.iter().filter_map(|placed| match &placed.module {
        AreaModule::Resident(resident) => Some(resident.npc),
        _ => None,
    })
}

pub fn warp_objects(content: &Content, area: AreaId) -> Vec<(u32, &ObjectDef)> {
    if !content.has_built::<Scripts>() {
        return Vec::new();
    }
    let scripts = content.built::<Scripts>();
    objects(placed(content, area))
        .filter(|(index, _)| {
            scripts
                .may_do(OwnerRow::of_object(area, *index))
                .reaching::<Warps>()
                .next()
                .is_some()
        })
        .collect()
}

/// The warp whose doorway `at` lies in, among the objects of the shown world.
pub fn warp_at(world: &mut World, at: Pos<Meters>) -> Option<Entity> {
    world
        .query::<(Entity, &AreaObject)>()
        .iter(world)
        .find(|(_, object)| object.warp.is_some() && object.shape.covers(at))
        .map(|(entity, _)| entity)
}

/// An area as the game plays it: its ground, where it walks and flies, what stands and shines on
/// it, and what it sounds like.
pub struct Area {
    pub size: Size<Meters>,
    pub max_slope: Degrees,
    /// The bodies its walk meshes keep clear of what blocks.
    pub bodies: Bodies,
    pub terrains: Vec<TerrainId>,
    pub waters: Vec<WaterId>,
    pub heights: Heightfield,
    pub paint: PaintMap,
    pub water: WaterSurface,
    pub water_map: WaterMap,
    /// How deep water carries nobody on foot, in a game whose water stops anyone.
    pub too_deep: Option<Meters>,
    /// The share of the downpour each stretch of the area gets; alike everywhere without one.
    pub rain: Option<ShareMap>,
    pub lighting: Lighting,
    pub climate: Climate,
    pub safe_zones: Vec<Rect<Meters>>,
    pub fogs: Vec<LaidFog>,
    pub soundscapes: Vec<SoundscapeDef>,
    /// Every prop standing in the area, scattered ones expanded.
    pub props: Vec<PropPlacement>,
    /// The props the area's objects stand as: each one's look where it stands, keeping the ground
    /// under it as a placed prop does.
    pub stood: Vec<PropPlacement>,
    /// The modules that stand the area's props, placed and scattered, in the order `props`
    /// expands them: what any chunk's props are worked out from alone.
    pub prop_sources: Vec<AreaModule>,
    /// Where each of `prop_sources` was written: the path of the node in the area's scene that is
    /// the module or brings it.
    pub prop_origins: Vec<String>,
    pub lights: Vec<PointLightDef>,
    /// The props among `props` that play vfx of their own or send ripples over the water, by
    /// their place there.
    pub playing: Vec<usize>,
    underfoot: Vec<walk::Underfoot>,
    /// What a step in each of `waters` sounds like.
    wading: Vec<SfxId>,
    footprints: Vec<Vec<Pos<Meters>>>,
    /// The footprints by the ground they cover, so a tile's mesh takes in only those reaching it.
    footprint_index: SpatialIndex,
    meshes: Arc<walk::Meshes>,
    modules_by_tile: HashMap<TileAt, Vec<u32>>,
    placements_by_tile: HashMap<TileAt, Vec<u32>>,
}

impl Area {
    pub fn safe(&self, at: Pos<Meters>) -> bool {
        self.safe_zones.iter().any(|zone| zone.contains(at))
    }

    /// The share of the downpour that falls at `at`, 0 none to 1 all of it.
    pub fn rain_share(&self, at: Pos<Meters>) -> f32 {
        self.rain
            .as_ref()
            .map_or(1.0, |rain| rain.share_at(Rect::from_size(self.size), at))
    }

    /// The ground everyone walks, as far as the tiles held.
    pub fn mesh(&self) -> HeldMesh {
        self.mesh_for(false, false)
    }

    /// The mesh an actor moves on, as far as the tiles held: the ground or, flying, the air, each
    /// cut around the safe zones for the wild; with no safe zone the wild moves on everyone's.
    pub fn mesh_for(&self, wild: bool, flies: bool) -> HeldMesh {
        walk::mesh(self, wild && !self.safe_zones.is_empty(), flies)
    }

    /// The mesh whatever draws `draught` of water floats on, as far as the tiles held: none for
    /// a draught no mount of the game floats at.
    pub fn water_mesh(&self, draught: Meters) -> Option<HeldMesh> {
        walk::water(self, draught)
    }

    /// The mesh of wherever water stands, as far as the tiles held: none in an area that spawns
    /// nothing on water.
    pub fn standing_water(&self) -> Option<HeldMesh> {
        walk::water(self, walk::ANY_WATER)
    }

    /// The ground everyone walks within a tile of `at`, laid for a look at one place of the area
    /// whether or not its ground is held there, and leaving what is held as it is.
    pub fn mesh_near(&self, at: Pos<Meters>) -> NavMesh {
        self.survey().around(at)
    }

    /// Whether everyone walks at `at`, once the ground there is held.
    pub fn walkable_if_held(&self, at: Pos<Meters>) -> Option<bool> {
        walk::holds(self, [self.ground_size().tile_at(at)]).then(|| self.mesh().walkable(at))
    }

    /// The tiles of ground held: those loaded, whose ground the meshes cover.
    pub fn held(&self) -> BTreeSet<TileAt> {
        walk::held(self)
    }

    /// Holds `tiles` for as long as the hold lives: loads their ground and lays the meshes over
    /// them.
    pub fn hold(&self, tiles: impl IntoIterator<Item = TileAt>) -> GroundHold {
        walk::hold(self, None, tiles)
    }

    /// Holds `tiles` in place of what `previous` held, letting go of the ground no hold wants any
    /// longer: what a holder that moves over the area, such as a character or a view, holds by.
    pub fn hold_instead(
        &self,
        previous: Option<GroundHold>,
        tiles: impl IntoIterator<Item = TileAt>,
    ) -> GroundHold {
        walk::hold(self, previous, tiles)
    }

    /// Whether every tile holding ground of `rect` is held, so the meshes cover it.
    pub fn holds(&self, rect: Rect<Meters>) -> bool {
        walk::holds(self, self.ground_size().tiles_within(rect))
    }

    /// The area's ground looked over a tile at a time, for a check of all of it.
    pub fn survey(&self) -> Survey<'_> {
        Survey::of(self)
    }

    /// Some may lie a little outside `rect`.
    pub fn footprints_within(&self, rect: Rect<Meters>) -> impl Iterator<Item = &[Pos<Meters>]> {
        self.footprint_index
            .within(rect)
            .map(|footprint| self.footprints[footprint].as_slice())
    }

    /// The tiles holding any ground within `reach` of `at`.
    pub fn tiles_around(&self, at: Pos<Meters>, reach: Meters) -> impl Iterator<Item = TileAt> {
        let around = Rect::new(
            Pos::new(at.x - reach.0, at.y - reach.0),
            Size::new(reach.0 * 2.0, reach.0 * 2.0),
        );
        self.ground_size().tiles_within(around)
    }

    pub fn ground_size(&self) -> GroundSize {
        self.heights.size()
    }

    /// The modules of the area's scene anchored in the tile at `at`, as indices among its placed
    /// modules: what comes to life when the tile is first reached.
    pub fn modules_in(&self, at: TileAt) -> &[u32] {
        self.modules_by_tile.get(&at).map_or(&[], Vec::as_slice)
    }

    /// The blueprint placements anchored in the tile at `at`, as indices among the area's.
    pub fn placements_in(&self, at: TileAt) -> &[u32] {
        self.placements_by_tile.get(&at).map_or(&[], Vec::as_slice)
    }

    /// What a foot put down at `at` sounds like: the water's surface where water stands over
    /// the ground there, and else the terrain painting most of the cell.
    pub fn footstep_at(&self, at: Pos<Meters>) -> Option<&SfxId> {
        let cell = self.size.grid().cell_under(at)?;
        let wading = self
            .wading
            .get(usize::from(self.water_map.at(at)))
            .filter(|_| self.water.depth_at(&self.heights, at).0 > 0.0);
        Some(wading.unwrap_or_else(|| &self.underfoot_of(self.paint.dominant(cell)).footstep))
    }

    pub fn fallback_spawn(&self) -> Pos<Meters> {
        self.mesh()
            .nearest(self.center())
            .unwrap_or_else(|| self.center())
    }

    pub fn center(&self) -> Pos<Meters> {
        Pos::new(
            (self.size.width / 2.0).floor(),
            (self.size.height / 2.0).floor(),
        )
    }

    /// What the terrain painted with `index` is underfoot; past the area's terrains, its last.
    fn underfoot_of(&self, index: u8) -> &walk::Underfoot {
        &self.underfoot[usize::from(index).min(self.underfoot.len() - 1)]
    }
}

pub struct Areas(Vec<Option<Arc<Area>>>);

pub fn load(content: &Content, area: AreaId) -> &Area {
    shared(content, area)
}

/// The area at `area` when it loaded, to keep beside the content it came from.
pub fn loaded(content: &Content, area: AreaId) -> Option<&Arc<Area>> {
    content
        .built::<Areas>()
        .0
        .get(area.index())
        .and_then(Option::as_ref)
}

/// The area as `load` gives it, to keep beside the content it came from.
pub fn shared(content: &Content, area: AreaId) -> &Arc<Area> {
    loaded(content, area).unwrap_or_else(|| panic!("area {} failed to load", area.path(content)))
}

/// What the editor pictures: an area, a blueprint on its own canvas, a prop standing in the
/// middle of a stage, the bare stage for a rig the editor stands on it, a water over each bed it
/// lies on, or a patch of a terrain, dry or under a metre of a water.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pictured {
    Area(AreaId),
    Blueprint(blueprint::BlueprintId),
    Prop(prop::PropId),
    Stage,
    Water(WaterId),
    Terrain(TerrainId, Option<WaterId>),
}

/// How wide the dry shore of a pictured water is, how far out from it the water's bed runs, and
/// how wide the band of each bed it is pictured over is.
pub const PICTURED_SHORE: Meters = Meters(4.0);
pub const PICTURED_REACH: Meters = Meters(28.0);
pub const PICTURED_BED: Meters = Meters(4.0);
/// How deep a terrain pictured under a water lies.
pub const PICTURED_UNDER: Meters = Meters(1.0);

/// How deep a pictured water of `clarity` stands `out` from its shore: nothing at the shore and
/// twice its clarity where its bed ends, deepening faster the farther out, so the shallows, where
/// a water's colour turns, take most of the picture.
pub fn pictured_depth(clarity: Meters, out: Meters) -> Meters {
    let share = (out.0 / PICTURED_REACH.0).clamp(0.0, 1.0);
    Meters(2.0 * clarity.0 * share * share)
}

/// How far out from its shore a pictured water of `clarity` stands `depth` deep.
pub fn pictured_out(clarity: Meters, depth: Meters) -> Meters {
    Meters(PICTURED_REACH.0 * (depth.0 / (2.0 * clarity.0)).clamp(0.0, 1.0).sqrt())
}

/// The areas that lay `water`.
pub fn laying(content: &Content, water: WaterId) -> impl Iterator<Item = AreaId> + '_ {
    content.ids::<AreaDef>().filter(move |&area| {
        content
            .table::<AreaDef>()
            .get(area)
            .is_some_and(|def| def.waters.contains(&water))
    })
}

/// The beds `water` is pictured over: every terrain of the areas that lay it, the home terrain
/// when none does.
pub fn pictured_beds(content: &Content, water: WaterId) -> Vec<TerrainId> {
    let mut beds = Vec::new();
    for area in laying(content, water) {
        let Some(def) = content.table::<AreaDef>().get(area) else {
            continue;
        };
        for terrain in &def.terrains {
            if !beds.contains(terrain) {
                beds.push(*terrain);
            }
        }
    }
    if beds.is_empty() {
        beds.extend(home_terrain(content));
    }
    beds.truncate(MAX_TERRAINS);
    beds
}

/// The flat ground a prop or a rig is pictured on, of the game's home terrain.
pub const PROP_STAGE: GroundSize = GroundSize {
    width: 8,
    height: 8,
};

/// What a bundle's scene makes, as the editor shows it.
pub struct Picture {
    /// The area the bundle makes with the nodes kept in it, as the game builds it.
    pub area: Arc<Area>,
    pub placed: Vec<Flattened>,
    pub placements: Vec<PlacedBlueprint>,
}

/// Which module nodes a picture draws, each named by its path as `blueprint::flatten_kept` names
/// it.
#[derive(Clone, Copy)]
pub enum Kept<'a> {
    All,
    Only(&'a dyn Fn(&[&str]) -> bool),
}

/// The picture of `pictured`, drawing the module nodes `kept`. A blueprint is pictured on its own
/// flat canvas with its holes filled by their defaults. An area with every node kept is the one
/// that loaded; its meshes cover the tiles held, as any area's do.
pub fn picture(content: &Content, pictured: Pictured, kept: Kept) -> Option<Picture> {
    if let (Pictured::Area(id), Kept::All) = (pictured, kept)
        && let Some(area) = loaded(content, id)
    {
        return Some(Picture {
            area: area.clone(),
            placed: placed(content, id).to_vec(),
            placements: placements(content, id).to_vec(),
        });
    }
    let mut errors = Vec::new();
    let keep: &dyn Fn(&[&str]) -> bool = match kept {
        Kept::All => &|_| true,
        Kept::Only(keep) => keep,
    };
    let (ground, flattening) = match pictured {
        Pictured::Area(id) => {
            let def = content.table::<AreaDef>().get(id)?;
            let ground = def.ground();
            (
                ground,
                blueprint::flatten_kept(content, &def.scene, keep, &mut errors),
            )
        }
        Pictured::Blueprint(id) => {
            let def = content.table::<BlueprintDef>().get(id)?;
            let ground = Ground {
                size: def.size,
                max_slope: UPRIGHT,
                terrains: vec![home_terrain(content)?],
                waters: Vec::new(),
                heights: Heightfield::flat(def.size, home_range(content)?),
                paint: PaintMap::plain(def.size),
                water: WaterSurface::dry(def.size, home_range(content)?),
                water_map: WaterMap::plain(def.size),
                rain: None,
                lighting: Lighting::day(),
                climate: Climate::always(Weather::breezy()),
            };
            (
                ground,
                blueprint::flatten_blueprint(content, id, keep, &mut errors),
            )
        }
        Pictured::Water(water) => {
            let def = content.table::<WaterDef>().get(water)?;
            let beds = pictured_beds(content, water);
            let lit_as =
                laying(content, water).find_map(|area| content.table::<AreaDef>().get(area));
            let size = GroundSize {
                width: (PICTURED_SHORE.0 + PICTURED_REACH.0) as u32,
                height: (beds.len() as u32 * PICTURED_BED.0 as u32).max(MIN_SIDE),
            };
            let range = HeightRange {
                lowest: Meters(-2.0 * def.clarity.0 - 1.0),
                highest: Meters(8.0),
            };
            let clarity = def.clarity;
            let band = PICTURED_BED.0 as u32 * SAMPLES_PER_METER;
            let last = beds.len().saturating_sub(1) as u32;
            let ground = Ground {
                size,
                max_slope: UPRIGHT,
                terrains: beds,
                waters: vec![water],
                heights: Heightfield::from_fn(size, range, |column, _| {
                    let out = column as f32 / SAMPLES_PER_METER as f32 - PICTURED_SHORE.0;
                    if out < 0.0 {
                        Meters(-out * SHORE_RISE)
                    } else {
                        Meters(-pictured_depth(clarity, Meters(out)).0)
                    }
                }),
                paint: PaintMap::from_fn(size, |_, row| (row / band).min(last) as u8),
                water: WaterSurface::from_fn(size, range, |_, _| Meters(0.0)),
                water_map: WaterMap::plain(size),
                rain: None,
                lighting: lit_as.map_or_else(Lighting::day, |area| area.lighting.clone()),
                climate: lit_as.map_or_else(
                    || Climate::always(Weather::breezy()),
                    |area| area.climate.clone(),
                ),
            };
            (
                ground,
                blueprint::flatten_kept(content, &[], keep, &mut errors),
            )
        }
        Pictured::Terrain(terrain, under) => {
            let range = HeightRange {
                lowest: Meters(-2.0 * PICTURED_UNDER.0),
                highest: Meters(2.0 * PICTURED_UNDER.0),
            };
            let bed = Meters(if under.is_some() {
                -PICTURED_UNDER.0
            } else {
                0.0
            });
            let ground = Ground {
                size: PROP_STAGE,
                max_slope: UPRIGHT,
                terrains: vec![terrain],
                waters: under.into_iter().collect(),
                heights: Heightfield::from_fn(PROP_STAGE, range, |_, _| bed),
                paint: PaintMap::plain(PROP_STAGE),
                water: match under {
                    Some(_) => WaterSurface::from_fn(PROP_STAGE, range, |_, _| Meters(0.0)),
                    None => WaterSurface::dry(PROP_STAGE, range),
                },
                water_map: WaterMap::plain(PROP_STAGE),
                rain: None,
                lighting: Lighting::day(),
                climate: Climate::always(Weather::breezy()),
            };
            (
                ground,
                blueprint::flatten_kept(content, &[], keep, &mut errors),
            )
        }
        Pictured::Prop(_) | Pictured::Stage => {
            let ground = Ground {
                size: PROP_STAGE,
                max_slope: UPRIGHT,
                terrains: vec![home_terrain(content)?],
                waters: Vec::new(),
                heights: Heightfield::flat(PROP_STAGE, home_range(content)?),
                paint: PaintMap::plain(PROP_STAGE),
                water: WaterSurface::dry(PROP_STAGE, home_range(content)?),
                water_map: WaterMap::plain(PROP_STAGE),
                rain: None,
                lighting: Lighting::day(),
                climate: Climate::always(Weather::breezy()),
            };
            let standing: Vec<scene::SceneNode> = match pictured {
                Pictured::Prop(id) => vec![scene::SceneNode::Module(scene::ModuleNode::new(
                    AreaModule::Prop(prop::PropPlacement {
                        prop: id,
                        at: PROP_STAGE.bounds().center(),
                        yaw: Degrees(0.0),
                        scale: 1.0,
                        sunk: Meters::default(),
                    }),
                ))],
                _ => Vec::new(),
            };
            (
                ground,
                blueprint::flatten_kept(content, &standing, keep, &mut errors),
            )
        }
    };
    let (area, _) = build_area(
        content,
        &ground,
        Bodies::of(content)?,
        &flattening.modules,
        &flattening.placements,
    );
    Some(Picture {
        area: Arc::new(area),
        placed: flattening.modules,
        placements: flattening.placements,
    })
}

/// The terrain the game begins on: what the initial spawn stands on, so a stage or a blueprint's
/// canvas looks like the game's own ground; any terrain when the spawn's area did not load.
pub fn home_terrain(content: &Content) -> Option<TerrainId> {
    let spawn = content.built::<GameSettings>().initial_spawn;
    let home = loaded(content, spawn.area).and_then(|area| {
        area.terrains
            .get(usize::from(area.paint.at(spawn.at)))
            .copied()
    });
    home.or_else(|| content.ids::<TerrainDef>().next())
}

/// The heights the game's first area spans, for a stage or a blueprint's canvas to stand within.
fn home_range(content: &Content) -> Option<HeightRange> {
    let spawn = content.built::<GameSettings>().initial_spawn;
    Some(content.table::<AreaDef>().get(spawn.area)?.height_range)
}

pub fn of(world: &World, entity: Entity) -> Option<&Area> {
    of_in(world.resource::<Content>(), world, entity)
}

pub fn of_in<'c>(content: &'c Content, world: &World, entity: Entity) -> Option<&'c Area> {
    let area = world.get::<AreaTag>(entity)?.area;
    Some(load(content, area))
}

fn flatten_all(loading: &mut Loading) {
    let content = loading.content();
    let mut placed: Vec<Vec<Flattened>> = content.ids::<AreaDef>().map(|_| Vec::new()).collect();
    let mut placements: Vec<Vec<PlacedBlueprint>> =
        content.ids::<AreaDef>().map(|_| Vec::new()).collect();
    let loaded: BTreeSet<blueprint::BlueprintId> = loading
        .rows::<BlueprintDef>(&content)
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    for (id, def) in loading.rows::<AreaDef>(&content) {
        let mut errors = Vec::new();
        let flattening = blueprint::flatten(&content, &def.scene, &mut errors);
        let tiles = def.size.each_tile().count();
        if flattening.modules.len() > blueprint::MAX_AREA_PARTS * tiles {
            errors.push(FlattenError {
                at: FlattenAt::Area(String::new()),
                problem: ContentProblem::new(
                    "too many parts",
                    format!("{} modules after flattening", flattening.modules.len()),
                    format!(
                        "at most {} a {} m tile of ground, {} for this area",
                        blueprint::MAX_AREA_PARTS,
                        TILE_SIDE.0,
                        blueprint::MAX_AREA_PARTS * tiles
                    ),
                ),
            });
        }
        let places_a_failed_blueprint = flattening
            .placements
            .iter()
            .any(|placed| !loaded.contains(&placed.blueprint));
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
}

fn build_all(loading: &mut Loading) {
    let content = loading.content();
    let mut areas: Vec<Option<Arc<Area>>> = content.ids::<AreaDef>().map(|_| None).collect();
    let mut reported: BTreeSet<(String, String)> = BTreeSet::new();
    let bodies = Bodies::of(&content);
    let mut sky = SkyTable::default();
    for (id, def) in loading.rows::<AreaDef>(&content) {
        let modules = placed(&content, id);
        for module in modules {
            let (file, pointer) = module.site(&content, id);
            let mut check = RowCheck::default();
            module.module.check(&pointer, &mut check);
            for (field, when) in module.module.whens() {
                if let Err(fault) = sky.follow(&content, when) {
                    check.error(
                        format!("{pointer}{field}"),
                        fault.rule,
                        format!("{when:?}"),
                        fault.expected,
                    );
                }
            }
            for error in check.into_errors() {
                if reported.insert((file.clone(), error.pointer.clone())) {
                    loading.json_error(&file, &error.pointer, error.problem);
                }
            }
        }
        let Some(bodies) = bodies else {
            continue;
        };
        let ground = def.ground();
        let mut check = RowCheck::default();
        check_bounds(&content, ground.size, modules, &mut check);
        let (area, budget) =
            build_area(&content, &ground, bodies, modules, placements(&content, id));
        check.extend(budget);
        check_placed(&content, id, modules, &mut check);
        if loading.rechecks(AreaDef::TABLE, id.path(&content)) {
            check_terrains(&content, id, def, loading);
            check_waters(&content, id, def, loading);
            check_footing(&content, id, &area, modules, &mut check);
        }
        let scene = scene_file(id.path(&content));
        for error in check.into_errors() {
            loading.json_error(&scene, &error.pointer, error.problem);
        }
        areas[id.index()] = Some(Arc::new(area));
    }
    loading.built(Areas(areas));
    loading.built(sky);
}

/// Every terrain the paint names must be listed: a check over every tile of the paint.
fn check_terrains(content: &Content, id: AreaId, def: &AreaDef, loading: &mut Loading) {
    let highest = usize::from(def.paint.highest());
    if highest >= def.terrains.len() {
        loading.json_error(
            fields_file(id.path(content)),
            "/terrains",
            ContentProblem::new(
                "paint names a missing terrain",
                format!("the paint names terrain {highest}"),
                format!("terrains 0 to {} listed", def.terrains.len() - 1),
            ),
        );
    }
}

/// Every water the water map names must be listed, in an area that keeps any water: a check over
/// every tile of the map. An area without water names none, and its map says nothing.
fn check_waters(content: &Content, id: AreaId, def: &AreaDef, loading: &mut Loading) {
    let highest = usize::from(def.water_map.highest());
    let wet = def.size.each_tile().any(|at| !def.water.is_blank_at(at));
    if (wet || highest > 0) && highest >= def.waters.len() {
        loading.json_error(
            fields_file(id.path(content)),
            "/waters",
            ContentProblem::new(
                "the water map names a missing water",
                format!("the water map names water {highest}"),
                match def.waters.len() {
                    0 => "a water listed for the area's water to be".to_string(),
                    listed => format!("waters 0 to {} listed", listed - 1),
                },
            ),
        );
    }
}

/// The ground an area is built on: what its config and parts say before any module stands on it.
pub(super) struct Ground {
    size: GroundSize,
    max_slope: Degrees,
    terrains: Vec<TerrainId>,
    waters: Vec<WaterId>,
    heights: Heightfield,
    paint: PaintMap,
    water: WaterSurface,
    water_map: WaterMap,
    rain: Option<ShareMap>,
    lighting: Lighting,
    climate: Climate,
}

/// The area a ground and its flattened modules make: where it walks, flies, is safe and sounds,
/// what stands and shines on it. With it, the problems of too much standing or shining there.
fn build_area(
    content: &Content,
    ground: &Ground,
    bodies: Bodies,
    modules: &[Flattened],
    placements: &[PlacedBlueprint],
) -> (Area, RowCheck) {
    let mut check = RowCheck::default();
    let tiles = ground.size.each_tile().count();
    let center = Pos::new(
        (ground.size.width as f32 / 2.0).floor(),
        (ground.size.height as f32 / 2.0).floor(),
    );
    let mut modules_by_tile: HashMap<TileAt, Vec<u32>> = HashMap::new();
    for (index, placed) in modules.iter().enumerate() {
        let anchor = match &placed.module {
            AreaModule::Object(object) => object.shape.center(),
            AreaModule::Resident(resident) => resident.at,
            AreaModule::Spawner(spawner) => spawner.within.map_or(center, |within| within.center()),
            _ => continue,
        };
        modules_by_tile
            .entry(ground.size.tile_at(anchor))
            .or_default()
            .push(index as u32);
    }
    let mut placements_by_tile: HashMap<TileAt, Vec<u32>> = HashMap::new();
    for (index, placement) in placements.iter().enumerate() {
        placements_by_tile
            .entry(ground.size.tile_at(placement.at))
            .or_default()
            .push(index as u32);
    }
    let safe_zones: Vec<Rect<Meters>> = modules
        .iter()
        .filter_map(|placed| match &placed.module {
            AreaModule::SafeZone(zone) => Some(zone.rect),
            _ => None,
        })
        .collect();
    let fogs: Vec<LaidFog> = modules
        .iter()
        .filter_map(|placed| match &placed.module {
            AreaModule::Fog(bank) => Some(bank.clone().laid(&ground.heights, &ground.water)),
            _ => None,
        })
        .collect();
    let mut props: Vec<PropPlacement> = Vec::new();
    let mut prop_sources: Vec<AreaModule> = Vec::new();
    let mut prop_origins: Vec<String> = Vec::new();
    let mut lights: Vec<PointLightDef> = Vec::new();
    for placed in modules {
        match &placed.module {
            AreaModule::Prop(prop) => {
                props.push(prop.clone());
                prop_sources.push(placed.module.clone());
                prop_origins.push(placed.origin.clone());
            }
            AreaModule::Scatter(scatter) => {
                props.extend(scatter.expand());
                prop_sources.push(placed.module.clone());
                prop_origins.push(placed.origin.clone());
            }
            AreaModule::Light(light) => {
                if lights.len() == MAX_LIGHTS * tiles {
                    check.error(
                        module_pointer(placed, ""),
                        "too many lights",
                        format!("light {} of the area", lights.len() + 1),
                        format!(
                            "at most {MAX_LIGHTS} lights a {} m tile of ground, {} for this area",
                            TILE_SIDE.0,
                            MAX_LIGHTS * tiles
                        ),
                    );
                }
                lights.push(light.clone());
            }
            _ => {}
        }
        if props.len() > MAX_PROPS * tiles
            && matches!(placed.module, AreaModule::Prop(_) | AreaModule::Scatter(_))
        {
            check.error(
                module_pointer(placed, ""),
                "too many props",
                format!("{} props standing in the area", props.len()),
                format!(
                    "at most {MAX_PROPS} props a {} m tile of ground, {} for this area, scattered ones counted",
                    TILE_SIDE.0,
                    MAX_PROPS * tiles
                ),
            );
        }
    }
    let stood: Vec<PropPlacement> = objects(modules)
        .filter_map(|(_, object)| object.standing())
        .collect();
    let footprints = walk::footprints(content, props.iter().chain(&stood));
    let mut footprint_index = SpatialIndex::new(TILE_SIDE);
    footprint_index.refile(
        footprints
            .iter()
            .map(|outline| Rect::from_points(outline.iter().copied())),
    );
    let playing = (0..props.len())
        .filter(|&index| {
            let prop = props[index].prop.get(content);
            !prop.effects.is_empty() || prop.ripples.is_some()
        })
        .collect();
    let area = Area {
        size: ground.size.meters(),
        max_slope: ground.max_slope,
        bodies,
        terrains: ground.terrains.clone(),
        waters: ground.waters.clone(),
        heights: ground.heights.clone(),
        paint: ground.paint.clone(),
        water: ground.water.clone(),
        water_map: ground.water_map.clone(),
        too_deep: content
            .try_built::<GameSettings>()
            .and_then(|settings| settings.wading)
            .map(|wading| wading.too_deep),
        rain: ground.rain.clone(),
        lighting: ground.lighting.clone(),
        climate: ground.climate.clone(),
        safe_zones,
        fogs,
        soundscapes: modules
            .iter()
            .filter_map(|placed| match &placed.module {
                AreaModule::Soundscape(soundscape) => Some(soundscape.clone()),
                _ => None,
            })
            .collect(),
        underfoot: walk::underfoot(content, &ground.terrains),
        wading: ground
            .waters
            .iter()
            .map(|water| water.get(content).surface.get(content).footstep)
            .collect(),
        footprints,
        footprint_index,
        props,
        stood,
        prop_sources,
        prop_origins,
        lights,
        playing,
        meshes: walk::Meshes::empty(ground.size.bounds(), walk::draughts(content, modules)),
        modules_by_tile,
        placements_by_tile,
    };
    (area, check)
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

fn check_bounds(content: &Content, size: GroundSize, modules: &[Flattened], check: &mut RowCheck) {
    let bounds = size.bounds();
    let outside = |at: Pos<Meters>| (!bounds.contains(at)).then_some(at);
    let corner_outside = |rect: Rect<Meters>| {
        outside(rect.min())
            .or_else(|| outside(rect.max() - Offset::new(0.001, 0.001)).map(|_| rect.max()))
    };
    for placed in modules {
        let lands = match &placed.module {
            AreaModule::Object(object) => corner_outside(object.shape.bounds()),
            AreaModule::Prop(prop) => outside(prop.at),
            AreaModule::Scatter(scatter) => corner_outside(scatter.region),
            AreaModule::Fog(bank) => corner_outside(bank.rect),
            AreaModule::Light(light) => outside(light.at),
            AreaModule::Resident(resident) => outside(resident.at),
            AreaModule::Spawner(Spawner {
                within: Some(within),
                ..
            }) => outside(within.min()),
            _ => None,
        };
        if let Some(at) = lands {
            check.error(
                module_pointer(placed, ""),
                "outside the terrain",
                format!(
                    "{} lands outside the terrain at {}, {}",
                    placed.describe(content),
                    at.x,
                    at.y
                ),
                format!(
                    "everything inside the {} × {} m ground",
                    size.width, size.height
                ),
            );
        }
    }
}

/// The area's warps, each with its display name.
fn named_warps(content: &Content, area: AreaId) -> Vec<(u32, &ObjectDef, String)> {
    warp_objects(content, area)
        .into_iter()
        .map(|(index, object)| (index, object, object.display_name(index)))
        .collect()
}

fn check_placed(content: &Content, id: AreaId, modules: &[Flattened], check: &mut RowCheck) {
    let mut names: Vec<(String, String)> = Vec::new();
    let mut seen_residents: Vec<NpcId> = Vec::new();
    for placed in modules {
        let site = |field: &str| module_pointer(placed, field);
        match &placed.module {
            AreaModule::Object(ObjectDef {
                name: Some(name), ..
            }) => {
                if let Some((_, first)) = names.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
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
            AreaModule::Resident(resident) => {
                if seen_residents.contains(&resident.npc) {
                    check.error(
                        site(""),
                        "resident twice",
                        format!("{} in {}", resident.npc.path(content), id.path(content)),
                        "each npc resident once per area",
                    );
                }
                seen_residents.push(resident.npc);
            }
            _ => {}
        }
    }
    let warps = named_warps(content, id);
    for (index, (object_index, object, name)) in warps.iter().enumerate() {
        for (_, other, other_name) in &warps[index + 1..] {
            if object.shape.overlaps(&other.shape) {
                check.error(
                    module_pointer(&modules[*object_index as usize], ""),
                    "warp objects overlap",
                    format!("{name} and {other_name}"),
                    "warp objects that never overlap",
                );
            }
        }
    }
}

/// Whether everything placed in the area that stands on its ground has ground to stand on.
fn check_footing(
    content: &Content,
    id: AreaId,
    area: &Area,
    modules: &[Flattened],
    check: &mut RowCheck,
) {
    let warps = named_warps(content, id);
    let in_warp = |at: Pos<Meters>| {
        warps
            .iter()
            .find(|(_, object, _)| object.shape.covers(at))
            .map(|(_, _, name)| name.clone())
    };
    let survey = area.survey();
    let nearest_ground = |at: Pos<Meters>| survey.nearest(at).map(describe_point);
    let beside = |object: &ObjectDef| {
        let stood = object.standing()?;
        let footprint = &stood.prop.get(content).footprint;
        let room = area.bodies.spacing().0;
        Some(
            footprint
                .bounds(stood.at, stood.yaw, stood.scale)?
                .inflate(room, room),
        )
    };
    let footing = |placed: &Flattened| match &placed.module {
        AreaModule::Object(object) => match object.shape {
            ObjectShape::Point(at) => Some(match beside(object) {
                Some(bounds) => SurveyQuestion::Covers {
                    bounds,
                    wild: false,
                },
                None => SurveyQuestion::Walkable(at),
            }),
            ObjectShape::Rect(_) => None,
        },
        AreaModule::Resident(resident) => Some(SurveyQuestion::Walkable(resident.at)),
        AreaModule::Spawner(spawner) => {
            let bounds = spawner.within.unwrap_or(area.size.bounds());
            Some(match spawner.on {
                SpawnSurface::Ground => SurveyQuestion::Covers {
                    bounds,
                    wild: spawner.keeps_wild(),
                },
                SpawnSurface::Water => SurveyQuestion::Wet(bounds),
            })
        }
        _ => None,
    };
    let ground_of = |object: &ObjectDef| match object.shape {
        ObjectShape::Point(at) => SurveyQuestion::Walkable(at),
        ObjectShape::Rect(rect) => SurveyQuestion::Covers {
            bounds: rect,
            wild: false,
        },
    };
    let questions: Vec<SurveyQuestion> = modules
        .iter()
        .filter_map(footing)
        .chain(warps.iter().map(|(_, object, _)| ground_of(object)))
        .collect();
    let answers = survey.answer(&questions);
    let mut answered = answers.iter().map(SurveyAnswer::yes);
    for placed in modules {
        if footing(placed).is_none() {
            continue;
        }
        let stands = answered.next().unwrap_or(true);
        let site = |field: &str| module_pointer(placed, field);
        match &placed.module {
            AreaModule::Object(object) => {
                if let ObjectShape::Point(at) = object.shape
                    && !stands
                {
                    check.problem(
                        site("/shape"),
                        ContentProblem::new(
                            "not walkable",
                            describe_point(at),
                            match beside(object) {
                                Some(_) => {
                                    "walkable ground beside the prop a point object stands as"
                                }
                                None => "a point object on walkable ground",
                            },
                        )
                        .near(nearest_ground(at)),
                    );
                }
            }
            AreaModule::Resident(resident) => {
                let key = resident.npc.path(content);
                if !stands {
                    check.problem(
                        site("/at"),
                        ContentProblem::new(
                            "not walkable",
                            format!("{key} {}", describe_point(resident.at)),
                            "a resident on walkable ground",
                        )
                        .near(nearest_ground(resident.at)),
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
            AreaModule::Spawner(spawner) if !stands => {
                let (rule, wanted) = match spawner.on {
                    SpawnSurface::Water => ("no water to spawn on", "water to spawn on"),
                    SpawnSurface::Ground if spawner.keeps_wild() => (
                        "no ground to spawn on",
                        "walkable ground outside every safe zone to spawn on",
                    ),
                    SpawnSurface::Ground => {
                        ("no ground to spawn on", "walkable ground to spawn on")
                    }
                };
                check.error(
                    site(""),
                    rule,
                    format!("{} in {}", spawner.named(content), id.path(content)),
                    wanted,
                );
            }
            _ => {}
        }
    }
    for ((object_index, _, name), covered) in warps.iter().zip(answered) {
        if !covered {
            check.error(
                module_pointer(&modules[*object_index as usize], ""),
                "warp object has no ground",
                name.clone(),
                "a warp object covering some walkable ground",
            );
        }
    }
}

/// Where a problem with an area's connectivity is reported.
#[derive(Clone, Debug, PartialEq)]
pub enum IslandSite {
    /// A node of an area's scene: the file and the JSON pointer.
    Scene(String, String),
    /// The game's initial spawn.
    Spawn,
}

/// Every resident, spawner, warp and the spawn standing on ground that nothing reaches, in the
/// areas `rechecks` asks for: no warp into its area and not the spawn connects to it. An area
/// nothing enters is left alone.
pub fn check_islands(
    content: &Content,
    spawn: Place,
    rechecks: impl Fn(AreaId) -> bool,
) -> Vec<(IslandSite, ContentProblem)> {
    let mut problems = Vec::new();
    for id in content.ids::<AreaDef>().filter(|id| rechecks(*id)) {
        let Some(area) = loaded(content, id) else {
            continue;
        };
        let survey = area.survey();
        let warps = warp_objects(content, id);
        let modules = placed(content, id);
        let asked = |placed: &Flattened| match &placed.module {
            AreaModule::Resident(resident) => Some(SurveyQuestion::IslandAt(resident.at)),
            AreaModule::Spawner(spawner) => Some(SurveyQuestion::Islands(
                spawner.within.unwrap_or(area.size.bounds()),
            )),
            AreaModule::Object(object) => Some(SurveyQuestion::Islands(object.shape.bounds())),
            _ => None,
        };
        let mut questions: Vec<SurveyQuestion> = modules.iter().filter_map(asked).collect();
        questions.push(SurveyQuestion::IslandAt(spawn.at));
        let answers: Vec<BTreeSet<SurveyIsland>> = survey
            .answer(&questions)
            .iter()
            .map(SurveyAnswer::islands)
            .collect();
        let spawn_islands = answers.last().cloned().unwrap_or_default();
        let mut by_module: Vec<Option<BTreeSet<SurveyIsland>>> = Vec::with_capacity(modules.len());
        let mut answered = answers.into_iter();
        for placed in modules.iter() {
            by_module.push(asked(placed).and_then(|_| answered.next()));
        }
        let mut entrances: Vec<(Option<u32>, SurveyIsland)> = warps
            .iter()
            .flat_map(|(index, _)| {
                by_module[*index as usize]
                    .iter()
                    .flatten()
                    .map(move |island| (Some(*index), *island))
            })
            .collect();
        if spawn.area == id {
            entrances.extend(spawn_islands.iter().map(|island| (None, *island)));
        }
        if entrances.is_empty() {
            continue;
        }
        let several = entrances
            .iter()
            .map(|(index, _)| *index)
            .collect::<BTreeSet<_>>()
            .len()
            > 1;
        let reached = |island: SurveyIsland, except: Option<u32>| {
            entrances.iter().any(|(index, entrance)| {
                except.is_none_or(|except| *index != Some(except)) && *entrance == island
            })
        };
        let reaches = |at: Pos<Meters>| {
            survey
                .island(at)
                .is_some_and(|island| reached(island, None))
        };
        let scene = scene_file(id.path(content));
        let mut report = |site: IslandSite, what: String, at: Pos<Meters>| {
            let nearest = survey.nearest_where(at, area.bodies.spacing(), reaches);
            problems.push((
                site,
                ContentProblem::new(
                    "unreachable",
                    format!("{what} at {}", describe_point(at)),
                    "ground a warp into the area or the spawn reaches",
                )
                .near(nearest.map(describe_point))
                .at_spot(nearest),
            ));
        };
        let unreached = |islands: &BTreeSet<SurveyIsland>, except: Option<u32>| {
            !islands.is_empty() && !islands.iter().any(|island| reached(*island, except))
        };
        let first_ground = |bounds: Rect<Meters>| survey.ground_within(bounds).first().copied();
        for (index, placed) in modules.iter().enumerate() {
            let site =
                |field: &str| IslandSite::Scene(scene.clone(), module_pointer(placed, field));
            let Some(islands) = &by_module[index] else {
                continue;
            };
            match &placed.module {
                AreaModule::Resident(resident) => {
                    if unreached(islands, None) {
                        report(
                            site("/at"),
                            resident.npc.path(content).to_owned(),
                            resident.at,
                        );
                    }
                }
                AreaModule::Spawner(spawner) => {
                    let region = spawner.within.unwrap_or(area.size.bounds());
                    if unreached(islands, None)
                        && let Some(first) = first_ground(region)
                    {
                        report(site(""), spawner.named(content), first);
                    }
                }
                AreaModule::Object(object) if several => {
                    let index = index as u32;
                    if warps.iter().any(|(warp, _)| *warp == index)
                        && unreached(islands, Some(index))
                        && let Some(first) = first_ground(object.shape.bounds())
                    {
                        report(site(""), object.display_name(index), first);
                    }
                }
                _ => {}
            }
        }
        if spawn.area == id
            && several
            && !spawn_islands.is_empty()
            && !entrances
                .iter()
                .any(|(index, entrance)| index.is_some() && spawn_islands.contains(entrance))
        {
            report(IslandSite::Spawn, "the initial spawn".to_owned(), spawn.at);
        }
    }
    problems
}

/// `place` when a character can be put there, else the nearest place of its area where one can:
/// on walkable ground, outside every warp object. None where the area has no such place.
pub fn standable(content: &Content, place: Place) -> Option<Place> {
    if check_place(content, place).is_ok() {
        return Some(place);
    }
    let area = loaded(content, place.area)?;
    let warps = warp_objects(content, place.area);
    let at = area
        .survey()
        .nearest_where(place.at, area.bodies.spacing(), |at| {
            !warps.iter().any(|(_, object)| object.shape.covers(at))
        })?;
    Some(Place {
        area: place.area,
        at,
    })
}

pub fn check_place(content: &Content, place: Place) -> Result<(), ContentProblem> {
    let Some(area) = loaded(content, place.area) else {
        return Ok(());
    };
    let survey = area.survey();
    let key = place.area.path(content);
    let warps = warp_objects(content, place.area);
    let problem = |rule: &'static str, found: String| {
        let nearest = survey
            .nearest_where(place.at, area.bodies.spacing(), |at| {
                !warps.iter().any(|(_, object)| object.shape.covers(at))
            })
            .map(describe_point);
        ContentProblem::new(
            rule,
            found,
            format!("a place on walkable ground of {key}, outside every warp object"),
        )
        .near(nearest)
    };
    if !survey.walkable(place.at) {
        return Err(problem(
            "not walkable",
            format!("{key} {}", describe_point(place.at)),
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

fn describe_point(at: Pos<Meters>) -> String {
    format!("{:.2}, {:.2}", at.x, at.y)
}

pub fn sixteenth(v: f32) -> f32 {
    (v * 16.0).round() / 16.0
}

/// Writes `file` as `encode` makes it, if `writer` wants it and there is anything to write.
fn put<Bytes: AsRef<[u8]>>(
    writer: &mut FileWriter,
    file: String,
    encode: impl FnOnce() -> Option<Bytes>,
) -> io::Result<()> {
    match writer.wants(&file).then(encode).flatten() {
        Some(bytes) => writer.file(&file, bytes.as_ref()),
        None => Ok(()),
    }
}
