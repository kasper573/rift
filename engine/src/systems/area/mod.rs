pub mod blueprint;
pub mod ground;
pub mod lighting;
pub mod object;
pub mod prop;
pub mod scene;
pub mod terrain;
pub mod transition;
pub mod walk;
pub mod warp;
pub mod world_map;

pub use blueprint::{BlueprintDef, Flattened, PlacedBlueprint, Placement};
pub use ground::{GroundDef, GroundId, SurfaceDef, SurfaceId, Walk};
pub use lighting::{Lighting, PointLightDef};
pub use object::{AreaObject, ObjectDef, ObjectShape};
pub use prop::{Footprint, PropDef, PropId, PropPlacement, Scatter};
pub use scene::SceneNode;
pub use terrain::{Heightfield, Paint, Terrain, TerrainSize};

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
use crate::core::math::{Offset, Pos, Rect, Size};
use crate::core::units::{Cell, CellPos, Meters, TilePos, TileRect, TileSize};
use crate::systems::actor::Pose;
use crate::systems::movement;
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
        .file::<GroundDef>()
        .named::<GroundDef>()
        .file::<SurfaceDef>()
        .named::<SurfaceDef>()
        .bundle::<BlueprintDef>()
        .bundle::<AreaDef>()
        .named::<AreaDef>()
        .file::<world_map::WorldMapDef>()
        .json_part(scene::json_part(AreaDef::TABLE))
        .json_part(scene::json_part(BlueprintDef::TABLE))
        .hook(LoadStep::Files, prop::check_fitted)
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

/// A place in the world: a terrain, the light it stands in, and a scene of modules on it. Its scene,
/// heights and paint are parts of its bundle.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AreaDef {
    pub name: String,
    pub terrain: Terrain,
    pub lighting: Lighting,
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
        let size = TerrainSize {
            width: 16,
            height: 12,
        };
        let ground: GroundId = near.file()?;
        Ok(AreaDef {
            name: near.name(),
            terrain: Terrain {
                size,
                materials: [ground; terrain::MATERIALS],
                max_slope: Degrees(38.0),
            },
            lighting: Lighting::day(),
            scene: Vec::new(),
            heights: Heightfield::flat(size),
            paint: Paint::plain(size),
        })
    }

    fn check(&self, cx: &mut RowCheck) {
        cx.text("/name", &self.name, 1, 60);
        self.terrain.check("/terrain", cx);
        self.lighting.check("/lighting", cx);
    }
}

impl NamedContent for AreaDef {
    fn shown_name(&self) -> &str {
        &self.name
    }
}

impl Bundle for AreaDef {
    const PARTS: &'static [PartSpec] = &[SCENE, terrain::HEIGHT, terrain::PAINT];
    const WRITTEN: &'static [PartSpec] = &[SCENE, terrain::HEIGHT, terrain::PAINT];

    fn read_parts(&mut self, parts: &mut Parts) {
        scene::read(&mut self.scene, &BTreeMap::new(), parts);
        let size = self.terrain.size;
        self.heights = read_part(
            parts,
            terrain::HEIGHT,
            || Heightfield::flat(size),
            |bytes| Heightfield::decode(size, bytes),
        );
        self.paint = read_part(
            parts,
            terrain::PAINT,
            || Paint::plain(size),
            |bytes| Paint::decode(size, bytes),
        );
    }

    fn write_parts(&self, bundle: &str, writer: &mut FileWriter) -> io::Result<()> {
        scene::write(&self.scene, bundle, writer)?;
        let part = |spec: PartSpec| format!("{bundle}/{}", spec.named("", "png"));
        if !self.heights.is_flat() {
            writer.file(&part(terrain::HEIGHT), &self.heights.encode())?;
        }
        if !self.paint.is_plain() {
            writer.file(&part(terrain::PAINT), &self.paint.encode())?;
        }
        Ok(())
    }
}

impl AreaDef {
    /// The area over a terrain of `size`, its heights and paint kept from the top-left corner.
    pub fn resized(&self, size: TerrainSize) -> AreaDef {
        let mut area = self.clone();
        area.terrain.size = size;
        area.heights = self.heights.resized(size);
        area.paint = self.paint.resized(size);
        area
    }

    pub fn size(&self) -> Size<Meters> {
        self.terrain.size.meters()
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

pub fn walkable(world: &World, tile: Pos<Meters>) -> bool {
    crate::systems::player::session::my_character(world)
        .map(|me| me.id())
        .and_then(|entity| of(world, entity))
        .is_some_and(|area| area.grid.walkable(tile))
}

/// An area as the game plays it: its ground, where it walks and flies, what stands and shines on
/// it, and what it sounds like.
pub struct Area {
    pub size: Size<Meters>,
    pub terrain: Terrain,
    pub heights: Heightfield,
    pub paint: Paint,
    pub lighting: Lighting,
    pub grid: movement::Grid,
    pub wild_grid: movement::Grid,
    pub airspace: movement::Grid,
    pub wild_airspace: movement::Grid,
    pub safe_zones: Vec<Rect<Meters>>,
    pub safe_cells: Vec<bool>,
    pub footsteps: Vec<Option<SfxId>>,
    pub soundscape: Vec<SoundscapeZone>,
    /// Every prop standing in the area, scattered ones expanded.
    pub props: Vec<PropPlacement>,
    pub lights: Vec<PointLightDef>,
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

    /// The depth the ground is drawn at: one terrain, under everything.
    pub fn sort_base(&self) -> f32 {
        0.0
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
                terrain: def.terrain.clone(),
                heights: def.heights.clone(),
                paint: def.paint.clone(),
                lighting: def.lighting.clone(),
            };
            (
                ground,
                blueprint::flatten_kept(content, &def.scene, keep, &mut errors),
            )
        }
        Pictured::Blueprint(id) => {
            let def = content.table::<BlueprintDef>().get(id)?;
            let first = content.ids::<GroundDef>().next()?;
            let ground = Ground {
                terrain: Terrain {
                    size: def.size,
                    materials: [first; terrain::MATERIALS],
                    max_slope: Degrees(terrain::MAX_SLOPE.0),
                },
                heights: Heightfield::flat(def.size),
                paint: Paint::plain(def.size),
                lighting: Lighting::day(),
            };
            (
                ground,
                blueprint::flatten_blueprint(content, id, keep, &mut errors),
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
    for (id, def) in loading.rows::<AreaDef>(&content) {
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
            terrain: def.terrain.clone(),
            heights: def.heights.clone(),
            paint: def.paint.clone(),
            lighting: def.lighting.clone(),
        };
        let mut check = RowCheck::default();
        check_bounds(&content, &ground.terrain, modules, &mut check);
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
struct Ground {
    terrain: Terrain,
    heights: Heightfield,
    paint: Paint,
    lighting: Lighting,
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
    let walk = walk::grids(
        content,
        &ground.terrain,
        &ground.heights,
        &ground.paint,
        &props,
        &safe_zones,
    );
    let size = ground.terrain.size.grid();
    let area = Area {
        size: ground.terrain.size.meters(),
        terrain: ground.terrain.clone(),
        heights: ground.heights.clone(),
        paint: ground.paint.clone(),
        lighting: ground.lighting.clone(),
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

fn check_bounds(content: &Content, terrain: &Terrain, modules: &[Flattened], check: &mut RowCheck) {
    let bounds = terrain.size.bounds();
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
                    "everything inside the {} × {} m terrain",
                    terrain.size.width, terrain.size.height
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
                            "a point object on walkable ground",
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
                            "a resident on walkable ground",
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
                        "walkable ground outside every safe zone to spawn on",
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
            format!("a place on walkable ground of {key}, outside every warp object"),
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
