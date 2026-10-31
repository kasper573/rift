pub mod blueprint;
pub mod lighting;
pub mod object;
pub mod prop;
pub mod prop_material;
pub mod render;
pub mod scene;
pub mod scenery;
pub mod terrain;
pub mod transition;
pub mod walk;
pub mod warp;
pub mod weather;
pub mod world_map;

pub use blueprint::{BlueprintDef, Flattened, PlacedBlueprint, Placement};
pub use lighting::{Lighting, PointLightDef};
pub use object::{AreaObject, ObjectDef, ObjectShape};
pub use prop::{Footprint, PropDef, PropId, PropPlacement, Scatter};
pub use scene::SceneNode;
pub use terrain::{SurfaceDef, SurfaceId, TerrainDef, TerrainId, Walk};
pub use weather::{DayCycle, Weather, Wind};

use std::collections::{BTreeMap, BTreeSet};
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
use crate::core::content::{
    Bundle, Content, ContentFile, ContentId, ContentProblem, ContentRegistry, FileWriter, LoadStep,
    Loading, NamedContent, Nearby, PartSpec, Parts, RowCheck, Shape, serial,
};
use crate::core::geometry::Degrees;
use crate::core::ground::{GroundSize, Heightfield, MAX_TERRAINS, MIN_SIDE, Paint};
use crate::core::math::{Offset, Pos, Rect, Size};
use crate::core::units::{GridDims, MeterSize, Meters};
use crate::systems::GameSettings;
use crate::systems::actor::Pose;
use crate::systems::movement;
use crate::systems::movement::NavMesh;
use crate::systems::npc::NpcId;
use crate::systems::script::{self, Condition, OwnerRow, Reach, RiftScripts, ScriptModule};
use blueprint::FlattenAt;
use lighting::MAX_LIGHTS;
use prop::MAX_PROPS;
use scene::{SCENE, scene_file};

pub type AreaId = ContentId<AreaDef>;

pub const MAX_POPULATION: u32 = 500;

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.replicate_once::<AreaTag>();
    object::register(app);
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
        .json_part(scene::json_part(AreaDef::TABLE))
        .json_part(scene::json_part(BlueprintDef::TABLE))
        .hook(LoadStep::Files, prop::check_fitted)
        .hook(LoadStep::Across, terrain::check_all)
        .hook(LoadStep::Across, terrain::check_pictures)
        .hook(LoadStep::Across, prop_material::check_all)
        .hook(LoadStep::Across, lighting::check_dome)
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

pub const MIN_SLOPE: Degrees = Degrees(10.0);
pub const MAX_SLOPE: Degrees = Degrees(60.0);

pub const HEIGHT: PartSpec = PartSpec::File {
    name: "height",
    exts: &["png"],
};
pub const PAINT: PartSpec = PartSpec::File {
    name: "paint",
    exts: &["png"],
};

/// A place in the world: a stretch of ground painted with terrains, the light it stands in, and a
/// scene of modules on it. Its scene, heights and paint are parts of its bundle.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AreaDef {
    pub name: String,
    pub size: GroundSize,
    /// How steep a slope still walks.
    pub max_slope: Degrees,
    /// The terrains the paint names by their index here: the first is the ground where nothing is
    /// painted. The editor adds a terrain as it is first painted.
    pub terrains: Vec<TerrainId>,
    pub lighting: Lighting,
    #[serde(default, skip_serializing_if = "Weather::is_still")]
    pub weather: Weather,
    #[serde(skip)]
    pub scene: Vec<SceneNode>,
    #[serde(skip)]
    pub heights: Heightfield,
    #[serde(skip)]
    pub paint: Paint,
}

impl ContentFile for AreaDef {
    const TABLE: &'static str = "area";
    const SHAPE: Shape = Shape::Bundle;

    fn fresh(near: &Nearby) -> Result<AreaDef, String> {
        let size = GroundSize {
            width: 16,
            height: 12,
        };
        Ok(AreaDef {
            name: near.name(),
            size,
            max_slope: Degrees(38.0),
            terrains: vec![near.file()?],
            lighting: Lighting::day(),
            weather: Weather::breezy(),
            scene: Vec::new(),
            heights: Heightfield::flat(size),
            paint: Paint::plain(size),
        })
    }

    fn check(&self, cx: &mut RowCheck) {
        cx.text("/name", &self.name, 1, 60);
        self.size.check("/size", MIN_SIDE, cx);
        cx.range(
            "/max_slope",
            self.max_slope.0,
            MIN_SLOPE.0,
            MAX_SLOPE.0,
            "degrees of slope",
        );
        cx.range(
            "/terrains",
            self.terrains.len(),
            1,
            MAX_TERRAINS,
            "terrains",
        );
        let highest = usize::from(self.paint.highest());
        if highest >= self.terrains.len() {
            cx.error(
                "/terrains",
                "paint names a missing terrain",
                format!("the paint names terrain {highest}"),
                format!("terrains 0 to {} listed", self.terrains.len() - 1),
            );
        }
        self.lighting.check("/lighting", cx);
        self.weather.check("/weather", cx);
    }
}

impl NamedContent for AreaDef {
    fn shown_name(&self) -> &str {
        &self.name
    }
}

impl Bundle for AreaDef {
    const PARTS: &'static [PartSpec] = &[SCENE, HEIGHT, PAINT];
    const WRITTEN: &'static [PartSpec] = &[SCENE, HEIGHT, PAINT];

    fn read_parts(&mut self, parts: &mut Parts) {
        scene::read(&mut self.scene, &BTreeMap::new(), parts);
        let size = self.size;
        self.heights = read_part(
            parts,
            HEIGHT,
            || Heightfield::flat(size),
            |bytes| Heightfield::decode(size, bytes),
        );
        self.paint = read_part(
            parts,
            PAINT,
            || Paint::plain(size),
            |bytes| Paint::decode(size, bytes),
        );
    }

    fn write_parts(&self, bundle: &str, writer: &mut FileWriter) -> io::Result<()> {
        scene::write(&self.scene, bundle, writer)?;
        let part = |spec: PartSpec| format!("{bundle}/{}", spec.named("", "png"));
        if !self.heights.is_flat() {
            writer.file(&part(HEIGHT), &self.heights.encode())?;
        }
        if !self.paint.is_plain() {
            writer.file(&part(PAINT), &self.paint.encode())?;
        }
        Ok(())
    }
}

impl AreaDef {
    /// The area over ground of `size`, its heights and paint kept from the top-left corner.
    pub fn resized(&self, size: GroundSize) -> AreaDef {
        let mut area = self.clone();
        area.size = size;
        area.heights = self.heights.resized(size);
        area.paint = self.paint.resized(size);
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
}

/// A part the bundle may leave out, `absent` then; one it keeps is `decode`d, and refused at its
/// file when it doesn't.
fn read_part<T>(
    parts: &mut Parts,
    spec: PartSpec,
    absent: impl FnOnce() -> T,
    decode: impl FnOnce(&[u8]) -> Result<T, ContentProblem>,
) -> T {
    let Some(file) = parts.file(spec) else {
        return absent();
    };
    let Some(bytes) = parts.bytes(file) else {
        return absent();
    };
    match decode(&bytes) {
        Ok(part) => part,
        Err(problem) => {
            parts.error(file, crate::core::content::ErrorPlace::WholeFile, problem);
            absent()
        }
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
    Prop,
    Scatter,
    Light,
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
            AreaModule::Prop(_) => AreaModuleKind::Prop,
            AreaModule::Scatter(_) => AreaModuleKind::Scatter,
            AreaModule::Light(_) => AreaModuleKind::Light,
            AreaModule::Blueprint(_) => AreaModuleKind::Blueprint,
            AreaModule::Population(_) => AreaModuleKind::Population,
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
            AreaModule::Prop(prop) => prop.at = moved(prop.at),
            AreaModule::Scatter(scatter) => scatter.region = moved_rect(scatter.region),
            AreaModule::Light(light) => light.at = moved(light.at),
            AreaModule::Blueprint(placement) => placement.at = moved(placement.at),
            AreaModule::Resident(resident) => resident.at = moved(resident.at),
            AreaModule::Population(population) => {
                population.roams = population.roams.map(moved_rect)
            }
            AreaModule::Script(_) => {}
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
            AreaModule::Prop(prop) => prop.check(pointer, cx),
            AreaModule::Scatter(scatter) => scatter.check(pointer, cx),
            AreaModule::Light(light) => light.check(pointer, cx),
            AreaModule::Resident(resident) => cx.range(
                format!("{pointer}/facing"),
                resident.facing.0,
                0.0,
                360.0,
                "degrees",
            ),
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
    pub shown: Option<Condition>,
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

/// The warp whose doorway `at` lies in, among the objects of the shown world.
pub fn warp_at(world: &mut World, at: Pos<Meters>) -> Option<Entity> {
    world
        .query::<(Entity, &AreaObject)>()
        .iter(world)
        .find(|(_, object)| object.warp.is_some() && object.shape.covers(at))
        .map(|(entity, _)| entity)
}

pub fn walkable(world: &World, at: Pos<Meters>) -> bool {
    crate::systems::player::session::my_character(world)
        .map(|me| me.id())
        .and_then(|entity| of(world, entity))
        .is_some_and(|area| area.mesh.walkable(at))
}

/// An area as the game plays it: its ground, where it walks and flies, what stands and shines on
/// it, and what it sounds like.
pub struct Area {
    pub size: Size<Meters>,
    pub max_slope: Degrees,
    pub terrains: Vec<TerrainId>,
    pub heights: Heightfield,
    pub paint: Paint,
    pub lighting: Lighting,
    pub weather: Weather,
    pub mesh: NavMesh,
    pub wild_mesh: NavMesh,
    pub airspace: NavMesh,
    pub wild_airspace: NavMesh,
    pub safe_zones: Vec<Rect<Meters>>,
    pub footsteps: Vec<Option<SfxId>>,
    pub soundscape: Vec<SoundscapeZone>,
    /// Every prop standing in the area, scattered ones expanded.
    pub props: Vec<PropPlacement>,
    pub lights: Vec<PointLightDef>,
}

impl Area {
    pub fn safe(&self, at: Pos<Meters>) -> bool {
        self.safe_zones.iter().any(|zone| zone.contains(at))
    }

    /// The mesh an actor moves on: the ground or, flying, the air, each cut around the safe zones
    /// for the wild.
    pub fn mesh_for(&self, wild: bool, flies: bool) -> &NavMesh {
        match (wild, flies) {
            (false, false) => &self.mesh,
            (true, false) => &self.wild_mesh,
            (false, true) => &self.airspace,
            (true, true) => &self.wild_airspace,
        }
    }

    /// The depth the ground is drawn at: one terrain, under everything.
    pub fn sort_base(&self) -> f32 {
        0.0
    }

    pub fn footstep_at(&self, at: Pos<Meters>) -> Option<&SfxId> {
        let i = self.size.grid().index_of(at)?;
        self.footsteps[i].as_ref()
    }

    pub fn fallback_spawn(&self) -> Pos<Meters> {
        self.mesh
            .nearest(self.center())
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
    content
        .built::<Areas>()
        .0
        .get(area.index())
        .and_then(Option::as_ref)
        .unwrap_or_else(|| panic!("area {} failed to load", area.path(content)))
}

/// A bundle the editor pictures: an area, a blueprint on its own canvas, a prop standing in the
/// middle of a stage, or the bare stage for a rig the editor stands on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pictured {
    Area(AreaId),
    Blueprint(blueprint::BlueprintId),
    Prop(prop::PropId),
    Stage,
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
/// flat canvas with its holes filled by their defaults.
pub fn picture(content: &Content, pictured: Pictured, kept: Kept) -> Option<Picture> {
    let mut errors = Vec::new();
    let keep: &dyn Fn(&[&str]) -> bool = match kept {
        Kept::All => &|_| true,
        Kept::Only(keep) => keep,
    };
    let (ground, flattening) = match pictured {
        Pictured::Area(id) => {
            let def = content.table::<AreaDef>().get(id)?;
            let ground = Ground {
                size: def.size,
                max_slope: def.max_slope,
                terrains: def.terrains.clone(),
                heights: def.heights.clone(),
                paint: def.paint.clone(),
                lighting: def.lighting.clone(),
                weather: def.weather,
            };
            (
                ground,
                blueprint::flatten_kept(content, &def.scene, keep, &mut errors),
            )
        }
        Pictured::Blueprint(id) => {
            let def = content.table::<BlueprintDef>().get(id)?;
            let ground = Ground {
                size: def.size,
                max_slope: MAX_SLOPE,
                terrains: vec![home_terrain(content)?],
                heights: Heightfield::flat(def.size),
                paint: Paint::plain(def.size),
                lighting: Lighting::day(),
                weather: Weather::breezy(),
            };
            (
                ground,
                blueprint::flatten_blueprint(content, id, keep, &mut errors),
            )
        }
        Pictured::Prop(_) | Pictured::Stage => {
            let ground = Ground {
                size: PROP_STAGE,
                max_slope: MAX_SLOPE,
                terrains: vec![home_terrain(content)?],
                heights: Heightfield::flat(PROP_STAGE),
                paint: Paint::plain(PROP_STAGE),
                lighting: Lighting::day(),
                weather: Weather::breezy(),
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
    let (area, _) = build_area(content, &ground, &flattening.modules);
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
    let home = content
        .built::<Areas>()
        .0
        .get(spawn.area.index())
        .and_then(Option::as_ref)
        .and_then(|area| {
            area.terrains
                .get(usize::from(area.paint.at(spawn.at)))
                .copied()
        });
    home.or_else(|| content.ids::<TerrainDef>().next())
}

pub fn of(world: &World, entity: Entity) -> Option<&Area> {
    of_in(world.resource::<Content>(), world, entity)
}

pub fn of_in<'c>(content: &'c Content, world: &World, entity: Entity) -> Option<&'c Area> {
    let area = world.get::<AreaTag>(entity)?.area;
    Some(load(content, area))
}

async fn flatten_all(loading: &mut Loading) {
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

async fn build_all(loading: &mut Loading) {
    let content = loading.content();
    let mut areas: Vec<Option<Arc<Area>>> = content.ids::<AreaDef>().map(|_| None).collect();
    let mut reported: BTreeSet<(String, String)> = BTreeSet::new();
    for (id, def) in loading.rows::<AreaDef>(&content) {
        loading.breathe().await;
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
        let ground = Ground {
            size: def.size,
            max_slope: def.max_slope,
            terrains: def.terrains.clone(),
            heights: def.heights.clone(),
            paint: def.paint.clone(),
            lighting: def.lighting.clone(),
            weather: def.weather,
        };
        let mut check = RowCheck::default();
        check_bounds(&content, ground.size, modules, &mut check);
        let (area, budget) = build_area(&content, &ground, modules);
        check.extend(budget);
        check_placed(&content, id, &area, modules, &mut check);
        let scene = scene_file(id.path(&content));
        for error in check.into_errors() {
            loading.json_error(&scene, &error.pointer, error.problem);
        }
        areas[id.index()] = Some(Arc::new(area));
    }
    loading.built(Areas(areas));
}

/// The ground an area is built on: what its config and parts say before any module stands on it.
pub(super) struct Ground {
    size: GroundSize,
    max_slope: Degrees,
    terrains: Vec<TerrainId>,
    heights: Heightfield,
    paint: Paint,
    lighting: Lighting,
    weather: Weather,
}

/// The area a ground and its flattened modules make: where it walks, flies, is safe and sounds,
/// what stands and shines on it. With it, the problems of too much standing or shining there.
fn build_area(content: &Content, ground: &Ground, modules: &[Flattened]) -> (Area, RowCheck) {
    let mut check = RowCheck::default();
    let safe_zones: Vec<Rect<Meters>> = modules
        .iter()
        .filter_map(|placed| match &placed.module {
            AreaModule::SafeZone(zone) => Some(zone.rect),
            _ => None,
        })
        .collect();
    let mut props: Vec<PropPlacement> = Vec::new();
    let mut lights: Vec<PointLightDef> = Vec::new();
    for placed in modules {
        match &placed.module {
            AreaModule::Prop(prop) => props.push(prop.clone()),
            AreaModule::Scatter(scatter) => props.extend(scatter.expand()),
            AreaModule::Light(light) => {
                if lights.len() == MAX_LIGHTS {
                    check.error(
                        module_pointer(placed, ""),
                        "too many lights",
                        format!("light {} of the area", lights.len() + 1),
                        format!("at most {MAX_LIGHTS} lights in an area"),
                    );
                }
                lights.push(light.clone());
            }
            _ => {}
        }
        if props.len() > MAX_PROPS
            && matches!(placed.module, AreaModule::Prop(_) | AreaModule::Scatter(_))
        {
            check.error(
                module_pointer(placed, ""),
                "too many props",
                format!("{} props standing in the area", props.len()),
                format!("at most {MAX_PROPS} props in an area, scattered ones counted"),
            );
        }
    }
    let walk = walk::meshes(content, ground, &props, &safe_zones);
    let area = Area {
        size: ground.size.meters(),
        max_slope: ground.max_slope,
        terrains: ground.terrains.clone(),
        heights: ground.heights.clone(),
        paint: ground.paint.clone(),
        lighting: ground.lighting.clone(),
        weather: ground.weather,
        mesh: walk.walk,
        wild_mesh: walk.wild,
        airspace: walk.airspace,
        wild_airspace: walk.wild_airspace,
        safe_zones,
        footsteps: walk.footsteps,
        soundscape: modules
            .iter()
            .filter_map(|placed| match &placed.module {
                AreaModule::Soundscape(soundscape) => Some(soundscape.zone()),
                _ => None,
            })
            .collect(),
        props,
        lights,
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
    let outside = |at: Pos<Meters>| !bounds.contains(at);
    let rect_outside =
        |rect: Rect<Meters>| outside(rect.min()) || outside(rect.max() - Offset::new(0.001, 0.001));
    for placed in modules {
        let site = |field: &str| module_pointer(placed, field);
        let what = placed.describe(content);
        let out = |check: &mut RowCheck, at: Pos<Meters>| {
            check.error(
                site(""),
                "outside the terrain",
                format!("{what} lands outside the terrain at {}, {}", at.x, at.y),
                format!(
                    "everything inside the {} × {} m ground",
                    size.width, size.height
                ),
            );
        };
        match &placed.module {
            AreaModule::Object(object) => {
                if rect_outside(object.shape.bounds()) {
                    out(check, object.shape.origin());
                }
            }
            AreaModule::Prop(prop) if outside(prop.at) => out(check, prop.at),
            AreaModule::Scatter(scatter) if rect_outside(scatter.region) => {
                out(check, scatter.region.origin)
            }
            AreaModule::Light(light) if outside(light.at) => out(check, light.at),
            AreaModule::Resident(resident) if outside(resident.at) => out(check, resident.at),
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
    let nearest_ground = |at: Pos<Meters>| area.mesh.nearest(at).map(describe_point);
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
                    && !area.mesh.walkable(at)
                {
                    check.problem(
                        site("/shape"),
                        ContentProblem::new(
                            "not walkable",
                            describe_point(at),
                            "a point object on walkable ground",
                        )
                        .near(nearest_ground(at)),
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
                if !area.mesh.walkable(resident.at) {
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
            AreaModule::Population(population) => {
                let has_ground = match population.roams {
                    Some(bounds) => area.wild_mesh.covers(bounds),
                    None => !area.wild_mesh.is_empty(),
                };
                if !has_ground {
                    check.error(
                        site(""),
                        "no wild ground",
                        format!("{} in {}", population.npc.path(content), id.path(content)),
                        "walkable ground outside every safe zone to spawn on",
                    );
                }
            }
            _ => {}
        }
    }
    for (index, (object_index, object, name)) in warps.iter().enumerate() {
        let pointer = module_pointer(&modules[*object_index as usize], "");
        let covered = match object.shape {
            ObjectShape::Point(at) => area.mesh.walkable(at),
            ObjectShape::Rect(rect) => area.mesh.covers(rect),
        };
        if !covered {
            check.error(
                pointer.clone(),
                "warp object has no ground",
                name.clone(),
                "a warp object covering some walkable ground",
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

/// Where a problem with an area's connectivity is reported.
#[derive(Clone, Debug, PartialEq)]
pub enum IslandSite {
    /// A node of an area's scene: the file and the JSON pointer.
    Scene(String, String),
    /// The game's initial spawn.
    Spawn,
}

/// Every resident, population, warp and the spawn standing on ground that nothing reaches: no warp
/// into its area and not the spawn connects to it. An area nothing enters is left alone.
pub fn check_islands(content: &Content, spawn: script::Place) -> Vec<(IslandSite, ContentProblem)> {
    let mut problems = Vec::new();
    for id in content.ids::<AreaDef>() {
        let Some(area) = content
            .built::<Areas>()
            .0
            .get(id.index())
            .and_then(Option::as_ref)
        else {
            continue;
        };
        let warps = warp_objects(content, id);
        let mut entrances: Vec<(Option<u32>, Pos<Meters>)> = warps
            .iter()
            .flat_map(|(index, object)| {
                area.mesh
                    .ground_within(object.shape.bounds())
                    .into_iter()
                    .map(move |at| (Some(*index), at))
            })
            .collect();
        if spawn.area == id {
            entrances.push((None, spawn.at));
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
        let reaches = |at: Pos<Meters>, except: Option<u32>| {
            entrances.iter().any(|(index, entrance)| {
                except.is_none_or(|except| *index != Some(except))
                    && area.mesh.connects(*entrance, at)
            })
        };
        let scene = scene_file(id.path(content));
        let mut report = |site: IslandSite, what: String, at: Pos<Meters>| {
            let nearest = area
                .mesh
                .nearest_where(at, movement::SPACING, |point| reaches(point, None));
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
        let modules = placed(content, id);
        for (index, placed) in modules.iter().enumerate() {
            let site =
                |field: &str| IslandSite::Scene(scene.clone(), module_pointer(placed, field));
            match &placed.module {
                AreaModule::Resident(resident) => {
                    if area.mesh.walkable(resident.at) && !reaches(resident.at, None) {
                        report(
                            site("/at"),
                            resident.npc.path(content).to_owned(),
                            resident.at,
                        );
                    }
                }
                AreaModule::Population(population) => {
                    let region = population.roams.unwrap_or(area.size.bounds());
                    let ground = area.mesh.ground_within(region);
                    if let Some(first) = ground.first()
                        && !ground.iter().any(|at| reaches(*at, None))
                    {
                        report(site(""), population.npc.path(content).to_owned(), *first);
                    }
                }
                AreaModule::Object(object) if several => {
                    let index = index as u32;
                    let ground = area.mesh.ground_within(object.shape.bounds());
                    if warps.iter().any(|(warp, _)| *warp == index)
                        && let Some(first) = ground.first()
                        && !ground.iter().any(|at| reaches(*at, Some(index)))
                    {
                        report(site(""), object.display_name(index), *first);
                    }
                }
                _ => {}
            }
        }
        if spawn.area == id
            && several
            && area.mesh.walkable(spawn.at)
            && !entrances
                .iter()
                .any(|(index, entrance)| index.is_some() && area.mesh.connects(*entrance, spawn.at))
        {
            report(IslandSite::Spawn, "the initial spawn".to_owned(), spawn.at);
        }
    }
    problems
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
            .mesh
            .nearest_where(place.at, movement::SPACING, |at| {
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
    if !area.mesh.walkable(place.at) {
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
