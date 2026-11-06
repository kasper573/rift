//! What stands in a world for an area: its ground as coloured meshes of its chunks, the covers
//! its terrains grow, its props on the ground, its point lights, its sun, the sky its sun lights
//! and the air over it, every one a child of one root. The client stands the area the player is
//! in; the editor stands the area it edits, on render layers of its own, and both light it alike
//! for the hour. The ground stands whole, finer the nearer the view, and the covers, props and
//! lights stand within the view's reach alone, a few a frame through `SceneryPlugin` as the view
//! moves, the props near the eye as scenes and the rest merged, so an area costs what the view
//! takes in, however far it stretches.

mod covers;
pub mod expand;
mod instances;
mod plan;
mod props;
mod regions;

pub use plan::{Eyes, Sight};
pub use props::prop_transform;

use covers::Covers;
use instances::Instances;
use plan::{Plan, Planner};
use props::{Filed, Props};
use regions::Regions;

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;
use std::sync::Arc;

use bevy::app::Propagate;
use bevy::camera::visibility::RenderLayers;
use bevy::ecs::component::Mutable;
use bevy::light::FogVolume;
use bevy::light::atmosphere::ScatteringMedium;
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, IoTaskPool, Task};

use super::lighting::DomeLook;
use super::prop::PropId;
use super::terrain::PieceShape;
use super::{Area, FogBank, Lighting, PointLightDef, Rainfall, Weather};
use crate::core::content::{Content, FileHash, FileHashes};
use crate::core::ground::{
    Heightfield, OVERVIEW_STRIDE, PaintMap, SAMPLES_PER_METER, TileAt, WaterSurface,
};
use crate::core::jobs::Handoff;
use crate::core::math::{Pos, Rect, Rng, Size};
use crate::core::render::camera::WorldPicture;
use crate::core::render::present::AreaGrade;
use crate::core::render::sample::FrameWork;
use crate::core::render::video::{VideoPreferences, WorldRenderer};
use crate::core::render::world::clouds::{CloudDrift, CloudSheet};
use crate::core::render::world::gpu::atlas::TerrainAtlas;
use crate::core::render::world::gpu::draw::{GroundDraw, GroundPatch, PATCH_CELLS};
use crate::core::render::world::gpu::upload;
use crate::core::render::world::haze::Haze;
use crate::core::render::world::lights::{self, Flicker};
use crate::core::render::world::lod::MeshLevels;
use crate::core::render::world::sky::{self, Daylight, SunLight};
use crate::core::render::world::terrain::{self, GroundLayer};
use crate::core::render::{CameraRig, Look, camera, world_point};
use crate::core::spatial::SpatialIndex;
use crate::core::units::{MeterSize, Meters};
use crate::core::vfx::cover::ShownWeather;
use crate::core::vfx::rain::{self as rain_vfx, RainField, RainMaterial};
use crate::core::vfx::render as vfx_render;
use crate::core::vfx::sky::{self as sky_vfx, Heavens, SkyMaterial, SkyWeather};
use crate::core::vfx::uniform::{ParamsUniform, SiteUniform, WeatherImage};

/// An area's scenery as it stands in a world: everything hangs under `root`, the terrain mesh is
/// `terrain`, `sun` is the directional light when the area has one, the sun by day and the full
/// moon by night, `sky` the dome its sky is painted on when the area paints one, and `rain` the
/// shells its rain falls on when its climate draws rain under an open sky.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scenery {
    pub root: Entity,
    pub terrain: Entity,
    pub sun: Option<Entity>,
    pub sky: Option<Entity>,
    pub air: Option<Entity>,
    pub rain: Option<Entity>,
}

/// Stands scenery as the view moves, a few pieces a frame, so the frames keep coming however much
/// the view takes in at once.
pub struct SceneryPlugin;

impl Plugin for SceneryPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TakenDown>()
            .init_resource::<Streams>()
            .add_systems(Update, (take_down, stream))
            .add_observer(props::level_ready_scene);
    }
}

/// Stands `area` in the world on `layers`: its sun, sky and air at once, and its ground, covers,
/// props and lights as the view comes to them, for the sight [`view_from`] gives it.
pub fn spawn(
    world: &mut World,
    content: &Content,
    area: &Arc<Area>,
    layers: RenderLayers,
) -> Scenery {
    let root = world
        .spawn((
            Transform::default(),
            Visibility::default(),
            Propagate(layers),
        ))
        .id();
    let sun = area.lighting.sun.map(|sun| {
        let clouds = world.resource::<CloudSheet>().over_sun();
        world
            .spawn((
                lights::sun(sun.color.color(), 0.0, sun.elevation, sun.azimuth),
                clouds,
                ChildOf(root),
            ))
            .id()
    });
    let look = *world.resource::<Look>();
    let humidity = humidity(&area.climate.first(), &look);
    let atmosphere = sky::atmosphere(
        &mut world.resource_mut::<Assets<ScatteringMedium>>(),
        area.lighting.sky,
        area.lighting.scattering(humidity),
        &look.sky,
    );
    world.spawn((atmosphere, ChildOf(root)));
    let mut dome = None;
    let mut air = None;
    let mut rain = None;
    if area.lighting.sky.is_air() {
        air = Some(
            world
                .spawn((
                    sky::air(Rect::zero(), Meters(0.0), Meters(0.0), 0.0),
                    ChildOf(root),
                ))
                .id(),
        );
        dome = area
            .lighting
            .dome
            .as_ref()
            .and_then(|look| spawn_dome(world, content, look, root));
        rain = area
            .climate
            .rain
            .as_ref()
            .and_then(|rainfall| spawn_rain(world, content, rainfall, root));
    }
    let terrain = spawn_terrain(world, content, area, &look, root);
    let covers = Covers::new(world, content, area);
    if let Some(mut draw) = world.get_mut::<GroundDraw>(terrain) {
        draw.sheets = covers.sheets().into();
    }
    let planner = {
        let (content, area) = (content.clone(), area.clone());
        let pieces = covers.pieces(&area.paint);
        Making::Task(AsyncComputeTaskPool::get().spawn(async move {
            let mut lights = SpatialIndex::new(PLACEMENT_CELL);
            lights.refile_spots(area.lights.iter().map(|light| light.at));
            Arc::new(Planner {
                regions: Regions::of(&area),
                filed: Filed::of(&content, &area),
                pieces,
                lights,
                area,
            })
        }))
    };
    let streaming = Streaming {
        content: content.clone(),
        area: area.clone(),
        heights: area.heights.clone(),
        paint: area.paint.clone(),
        water: area.water.clone(),
        terrain,
        covers,
        props: Props::default(),
        instances: Instances::default(),
        renderer: world
            .get_resource::<VideoPreferences>()
            .map_or_else(WorldRenderer::default, |video| video.world_renderer),
        planner,
        sight: None,
        planning: None,
        plan: None,
        whole: false,
        stood: Stood::default(),
        drawn: None,
        missing: 0,
        loading: HashMap::default(),
    };
    world.resource_mut::<Streams>().0.insert(root, streaming);
    Scenery {
        root,
        terrain,
        sun,
        sky: dome,
        air,
        rain,
    }
}

/// Tells the scenery the sight it stands for: the covers, props and lights within its reach, a
/// cover's pieces and a prop only where they span a pixel from the nearest its eyes may come, a
/// prop's meshes no finer than shown from there. The ground stands whole, finer the nearer the
/// ground the sight looks at. What stands follows over the frames.
pub fn view_from(world: &mut World, scenery: Scenery, sight: Sight) {
    if let Some(streaming) = world
        .resource_mut::<Streams>()
        .bypass_change_detection()
        .0
        .get_mut(&scenery.root)
    {
        streaming.sight = Some(sight);
    }
}

/// The files the scenery of an area is made from beyond the area itself, each with what it held
/// when read: its terrains, their covers' shaders, the props it stands and strews, and their
/// models. Scenery stood from equal sources looks alike, so it need not stand again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sources(Vec<(String, FileHash)>);

impl Sources {
    pub fn of(content: &Content, area: &Area) -> Sources {
        let mut files: BTreeSet<&str> = BTreeSet::new();
        let mut props = props_of(area);
        for terrain in &area.terrains {
            files.insert(terrain.path(content));
            let Some(cover) = &terrain.get(content).cover else {
                continue;
            };
            files.insert(cover.shader.path(content));
            props.extend(cover.pieces.iter().filter_map(|piece| match piece.shape {
                PieceShape::Prop { prop } => Some(prop),
                PieceShape::Blade { .. } => None,
            }));
        }
        if let Some(dome) = &area.lighting.dome {
            files.insert(dome.shader.path(content));
        }
        if let Some(rain) = &area.climate.rain {
            files.insert(rain.shader.path(content));
        }
        for prop in props {
            let def = prop.get(content);
            files.insert(prop.path(content));
            files.insert(def.model.as_str());
            for material in def.materials.values() {
                files.insert(material.path(content));
                files.insert(material.get(content).shader.path(content));
            }
        }
        let hashes = &content.built::<FileHashes>().0;
        let held: BTreeMap<String, FileHash> = files
            .iter()
            .flat_map(|&file| {
                let end = file_bound(file);
                hashes
                    .range::<str, _>((Bound::Included(file), Bound::Excluded(end.as_str())))
                    .filter(move |(path, _)| {
                        path.strip_prefix(file)
                            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
                    })
            })
            .map(|(path, hash)| (path.clone(), hash.clone()))
            .collect();
        Sources(held.into_iter().collect())
    }
}

/// The path just past every part of `file`'s bundle in path order: after `file` itself and
/// `file/…` comes the next character above the separator.
fn file_bound(file: &str) -> String {
    format!("{file}0")
}

/// The props standing in the area, each once.
fn props_of(area: &Area) -> Vec<PropId> {
    let mut props: Vec<PropId> = area.props.iter().map(|placed| placed.prop).collect();
    props.sort_unstable();
    props.dedup();
    props
}

/// Whether every shader `area`'s covers and props draw with is compiled, so the scenery can stand
/// whole: the shaders follow the content into the world a frame behind it.
pub fn shaders_ready(world: &World, content: &Content, area: &Area) -> bool {
    let covers = area.terrains.iter().all(|terrain| {
        terrain
            .get(content)
            .cover
            .as_ref()
            .is_none_or(|cover| vfx_render::compiled_shader(world, cover.shader).is_some())
    });
    let sky = area
        .lighting
        .dome
        .as_ref()
        .is_none_or(|dome| vfx_render::compiled_shader(world, dome.shader).is_some());
    let rain = area
        .climate
        .rain
        .as_ref()
        .is_none_or(|rain| vfx_render::compiled_shader(world, rain.shader).is_some());
    covers
        && sky
        && rain
        && props_of(area).iter().all(|prop| {
            prop.get(content).materials.values().all(|material| {
                vfx_render::compiled_shader(world, material.get(content).shader).is_some()
            })
        })
}

/// Whether any scenery has yet to stand what its sight takes in.
pub fn standing(world: &World) -> bool {
    world
        .resource::<Streams>()
        .0
        .values()
        .any(|streaming| !streaming.stands())
}

/// How many chunks of ground and regions of scenery the sights take in that have yet to stand.
pub fn missing(world: &World) -> u32 {
    world
        .resource::<Streams>()
        .0
        .values()
        .map(|streaming| streaming.missing)
        .sum()
}

/// How many props `scenery` holds in its arena on the GPU.
pub fn arena_props(world: &World, scenery: Scenery) -> u32 {
    world
        .resource::<Streams>()
        .0
        .get(&scenery.root)
        .map_or(0, |streaming| streaming.instances.props())
}

/// How many patches of ground are drawn for the sights.
pub fn stood_chunks(world: &World) -> u32 {
    world
        .resource::<Streams>()
        .0
        .values()
        .filter_map(|streaming| world.get::<GroundDraw>(streaming.terrain))
        .map(|draw| draw.patches.len() as u32)
        .sum()
}

/// Whether `scenery` stands all its sight takes in.
pub fn stood(world: &World, scenery: Scenery) -> bool {
    world
        .resource::<Streams>()
        .0
        .get(&scenery.root)
        .is_some_and(Streaming::stands)
}

/// How many sceneries stand.
pub fn sceneries(world: &World) -> usize {
    world.resource::<Streams>().0.len()
}

/// What every scenery standing stands as its sight moves, by its root.
#[derive(Resource, Default)]
struct Streams(HashMap<Entity, Streaming>);

/// What an area's scenery stands as its sight moves: the area and what it's drawn from, the
/// sight, the plan for it, and what stands now.
struct Streaming {
    content: Content,
    area: Arc<Area>,
    heights: Heightfield,
    paint: PaintMap,
    water: WaterSurface,
    terrain: Entity,
    covers: Covers,
    props: Props,
    instances: Instances,
    /// The way its props are drawn, as the video preferences asked when it stood.
    renderer: WorldRenderer,
    planner: Making<Arc<Planner>>,
    sight: Option<Sight>,
    /// The plan being made, for the widened sight it is made for.
    planning: Option<(Sight, Task<Plan>)>,
    plan: Option<Arc<Plan>>,
    /// Whether all the plan takes in stands, so a sight that stays within it costs nothing to
    /// stream.
    whole: bool,
    stood: Stood,
    /// The plan whose ground the terrain draws.
    drawn: Option<Arc<Plan>>,
    /// How much of the ground and how many regions of scenery the plan takes in have yet to stand.
    missing: u32,
    /// The tiles of ground being loaded for what stands on them.
    loading: HashMap<TileAt, Task<()>>,
}

impl Streaming {
    /// Whether all the latest sight takes in stands.
    fn stands(&self) -> bool {
        self.whole
            && self.sight.is_some()
            && self.plan.as_ref().map(|plan| plan.seen.sight) == self.sight.map(Sight::widened)
    }

    /// Whether streaming has nothing to do until the sight moves.
    fn idle(&self) -> bool {
        self.sight.is_none() || self.stands()
    }
}

#[derive(Default)]
struct Stood {
    lights: HashMap<usize, Entity>,
}

/// A chunk of the ground at a level of detail: the node of the quadtree over the samples it is,
/// by its level (0 the leaves) and its place among the nodes of that level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Node {
    level: u32,
    column: u32,
    row: u32,
}

/// The samples along a leaf node's side: a node is drawn as a patch.
const LEAF_SAMPLES: u32 = PATCH_CELLS;
/// How near the view a node of the ground or a region of the scenery must lie, in its own sides,
/// to be cut into finer ones.
const SPLIT_WITHIN: f32 = 1.5;
/// The level from which a node's stride is the overview's, so it draws from the overview alone;
/// a finer node wants the tiles of ground under it in detail.
const OVERVIEW_LEVEL: u32 = OVERVIEW_STRIDE.trailing_zeros();
/// How placements are filed to be found near the view, and the side of the leaves of the
/// quadtree the props stand over.
const PLACEMENT_CELL: Meters = Meters(32.0);
/// How much stands in one frame, and among the props how many whose model no earlier prop asked
/// for: a model's load falls on the frames after the prop that starts it.
const COVERS_PER_FRAME: usize = 16;
const PROPS_PER_FRAME: usize = 16;
const FRESH_MODELS_PER_FRAME: usize = 1;
const MERGES_PER_FRAME: usize = 4;
const PARTS_TAKEN_DOWN_PER_FRAME: usize = 2000;

fn stream(world: &mut World) {
    let simplified = world.is_resource_changed::<MeshLevels>();
    if !simplified && world.resource::<Streams>().0.values().all(Streaming::idle) {
        return;
    }
    world.resource_scope(|world, mut streams: Mut<Streams>| {
        for (&root, streaming) in streams.bypass_change_detection().0.iter_mut() {
            replan(world, streaming);
            stream_plan(world, root, streaming, simplified);
        }
    });
}

/// Plans for the sight anew once it has moved out of the steps the plan was made for, letting go
/// of a plan being made for a sight it has since left, and takes the plan once it is made.
fn replan(world: &mut World, streaming: &mut Streaming) {
    let Some(sight) = streaming.sight.map(Sight::widened) else {
        return;
    };
    let Some(planner) = streaming.planner.made().cloned() else {
        return;
    };
    if let Some((planned, task)) = streaming.planning.take()
        && planned == sight
    {
        match Handoff::of(world).take(task) {
            Ok(plan) => {
                streaming.plan = Some(Arc::new(plan));
                streaming.whole = false;
            }
            Err(task) => streaming.planning = Some((planned, task)),
        }
        return;
    }
    if streaming
        .plan
        .as_ref()
        .is_some_and(|plan| plan.seen.sight == sight)
    {
        return;
    }
    let heights = streaming.heights.clone();
    let task = AsyncComputeTaskPool::get().spawn(async move { planner.plan(sight, &heights) });
    streaming.planning = Some((sight, task));
    if let Some(mut work) = world.get_resource_mut::<FrameWork>() {
        work.plans += 1;
    }
}

/// Stands what the plan takes in, as much as a frame's budget allows, and takes down what it
/// left behind once what replaces it stands.
fn stream_plan(world: &mut World, root: Entity, streaming: &mut Streaming, simplified: bool) {
    let (Some(planner), Some(plan)) = (streaming.planner.made().cloned(), streaming.plan.clone())
    else {
        return;
    };
    if streaming.whole {
        if simplified {
            props::level_props(world, streaming, &plan.seen);
        }
        return;
    }
    streaming
        .loading
        .retain(|_, task| check_ready(task).is_none());
    let mut budget = Budget::new(world);
    let ground_missing = draw_ground(world, streaming, &plan);
    if ground_missing > 0 {
        budget.defer();
    }
    let props_missing = match streaming.renderer {
        WorldRenderer::Legacy => {
            let unstood = props::stand(world, streaming, &planner, &plan, root, &mut budget);
            props::clear(world, streaming, &planner, &plan, &unstood);
            if !unstood.is_empty() {
                budget.defer();
            }
            unstood.len() as u32
        }
        WorldRenderer::Gpu => {
            let missing = instances::stand(world, streaming, &plan, root, &mut budget);
            instances::clear(world, streaming, &plan, root);
            missing
        }
    };
    let covers_unstood = covers::stand(world, streaming, &planner, &plan, root, &mut budget);
    streaming.missing = ground_missing + props_missing + covers_unstood.len() as u32;
    covers::clear(world, streaming, &planner, &plan, &covers_unstood);
    if !covers_unstood.is_empty() {
        budget.defer();
    }
    props::level_props(world, streaming, &plan.seen);
    stream_lights(world, streaming, &plan, root);
    streaming.whole = !budget.spent;
}

/// What a frame may still stand: the meshes it stands go to the GPU as the upload stage sends
/// them, so new ones are made only while it has room, while the entities and scenes it spawns
/// cost the frame itself.
struct Budget {
    covers: usize,
    props: usize,
    fresh: usize,
    merges: usize,
    /// Whether the upload stage takes more meshes this frame.
    staging: bool,
    /// Whether the frame left something to stand.
    spent: bool,
}

impl Budget {
    fn new(world: &World) -> Budget {
        Budget {
            covers: COVERS_PER_FRAME,
            props: PROPS_PER_FRAME,
            fresh: FRESH_MODELS_PER_FRAME,
            merges: MERGES_PER_FRAME,
            staging: upload::has_room(world),
            spent: false,
        }
    }

    /// Leaves something to stand on a later frame, as it waits on what is being made or loaded.
    fn defer(&mut self) {
        self.spent = true;
    }

    /// Whether the frame makes or stands meshes for the GPU: once the upload stage has room for
    /// them.
    fn stage(&mut self) -> bool {
        if !self.staging {
            self.spent = true;
        }
        self.staging
    }

    fn take(&mut self, left: fn(&mut Budget) -> &mut usize) -> bool {
        let left = left(self);
        if *left == 0 {
            self.spent = true;
            return false;
        }
        *left -= 1;
        true
    }
}

/// Draws the ground `plan` wants, its nodes finer than the overview in detail as the terrain's
/// atlas takes their tiles in, those any eye may come nearest first; how much of it has yet to
/// reach the GPU.
fn draw_ground(world: &mut World, streaming: &mut Streaming, plan: &Arc<Plan>) -> u32 {
    let terrain = streaming.terrain;
    if streaming
        .drawn
        .as_ref()
        .is_none_or(|drawn| !Arc::ptr_eq(drawn, plan))
    {
        let samples = streaming.heights.samples();
        let patches = patches_of(&plan.ground, samples);
        let size = streaming.heights.size();
        let nearest_eye = |tile: TileAt| {
            let bounds = tile.rect(size);
            let ground = streaming.heights.height_at(bounds.center());
            plan.seen.over(bounds, ground, ground)
        };
        let mut wanted: Vec<(f32, TileAt)> = plan
            .ground
            .iter()
            .filter(|node| node.level < OVERVIEW_LEVEL)
            .flat_map(|node| size.tiles_within(node_rect(*node).bounds()))
            .collect::<HashSet<TileAt>>()
            .into_iter()
            .map(|tile| (nearest_eye(tile), tile))
            .collect();
        wanted.sort_by(|(a, at), (b, bt)| a.total_cmp(b).then(at.cmp(bt)));
        if let Some(mut draw) = world.get_mut::<GroundDraw>(terrain) {
            draw.patches = patches.into();
        }
        if let Some(mut atlas) = world.get_mut::<TerrainAtlas>(terrain) {
            atlas.want(wanted.into_iter().map(|(_, tile)| tile));
        }
        streaming.drawn = Some(plan.clone());
    }
    world
        .get::<TerrainAtlas>(terrain)
        .map_or(0, TerrainAtlas::missing)
}

/// Whether every tile of ground under `bounds` is loaded, each not yet being loaded set loading on
/// a task. What stands on the ground waits for it: until a tile loads, its values are read from
/// the overview, and what stood from those would stay as it stood.
fn ground_loaded(streaming: &mut Streaming, bounds: Rect<Meters>) -> bool {
    let (heights, paint, water) = (&streaming.heights, &streaming.paint, &streaming.water);
    let mut ready = true;
    for tile in heights.size().tiles_within(bounds) {
        if heights.is_loaded(tile) && paint.is_loaded(tile) && water.is_loaded(tile) {
            continue;
        }
        ready = false;
        if streaming.loading.contains_key(&tile) {
            continue;
        }
        let (heights, paint, water) = (heights.clone(), paint.clone(), water.clone());
        let task = IoTaskPool::get().spawn(async move {
            for problem in [
                heights.load(tile).err(),
                paint.load(tile).err(),
                water.load(tile).err(),
            ]
            .into_iter()
            .flatten()
            {
                warn!("ground tile {} doesn't load: {problem}", tile.key());
            }
        });
        streaming.loading.insert(tile, task);
    }
    ready
}

/// The patches `nodes` are drawn as over a ground of `samples` across and down, each meeting a
/// coarser neighbour on its points.
fn patches_of(nodes: &[Node], samples: (u32, u32)) -> Vec<GroundPatch> {
    let laid: HashSet<Node> = nodes.iter().copied().collect();
    let top = nodes.iter().map(|node| node.level).max().unwrap_or(0);
    let last = (samples.0.saturating_sub(1), samples.1.saturating_sub(1));
    let stride_at = |node: Node, sample: Option<(u32, u32)>| {
        let own = 1 << node.level;
        let Some((column, row)) = sample.filter(|&(column, row)| column < last.0 && row < last.1)
        else {
            return own;
        };
        (node.level..=top)
            .map(|level| Node {
                level,
                column: column / (LEAF_SAMPLES << level),
                row: row / (LEAF_SAMPLES << level),
            })
            .find(|beside| laid.contains(beside))
            .map_or(own, |beside| 1 << beside.level)
    };
    nodes
        .iter()
        .map(|&node| {
            let NodeRect { first, side } = node_rect(node);
            let middle = (first.0 + side / 2, first.1 + side / 2);
            let beside = [
                first.0.checked_sub(1).map(|column| (column, middle.1)),
                first.1.checked_sub(1).map(|row| (middle.0, row)),
                Some((first.0 + side, middle.1)),
                Some((middle.0, first.1 + side)),
            ]
            .map(|sample| stride_at(node, sample));
            GroundPatch::new(first, 1 << node.level, beside)
        })
        .collect()
}

/// The nodes of the quadtree over the ground's samples that stand for a view looking at ground
/// within `off` of `at`: every node is cut in four while it may lie within `SPLIT_WITHIN` of its
/// sides from that ground and is wider than a leaf; what remains covers the ground once over,
/// finer the nearer the view.
fn nodes_for(heights: &Heightfield, at: Pos<Meters>, off: f32) -> Vec<Node> {
    let (columns, rows) = heights.samples();
    let cells = (columns.max(rows).saturating_sub(1)).max(1);
    let levels = (cells.div_ceil(LEAF_SAMPLES))
        .next_power_of_two()
        .trailing_zeros();
    let mut nodes = Vec::new();
    let mut open = vec![Node {
        level: levels,
        column: 0,
        row: 0,
    }];
    while let Some(node) = open.pop() {
        let rect = node_rect(node);
        if rect.first.0 >= columns - 1 || rect.first.1 >= rows - 1 {
            continue;
        }
        let bounds = rect.bounds();
        let near = distance_to(bounds, at) - off < bounds.size.width * SPLIT_WITHIN;
        if node.level > 0 && near {
            for (dc, dr) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                open.push(Node {
                    level: node.level - 1,
                    column: node.column * 2 + dc,
                    row: node.row * 2 + dr,
                });
            }
        } else {
            nodes.push(node);
        }
    }
    nodes
}

/// Where a node's samples begin and how many lie along its side.
struct NodeRect {
    first: (u32, u32),
    side: u32,
}

fn node_rect(node: Node) -> NodeRect {
    let side = LEAF_SAMPLES << node.level;
    NodeRect {
        first: (node.column * side, node.row * side),
        side,
    }
}

impl NodeRect {
    /// The ground the node's samples span.
    fn bounds(&self) -> Rect<Meters> {
        let side = self.side as f32 / SAMPLES_PER_METER as f32;
        Rect::new(
            Heightfield::sample_pos(self.first.0, self.first.1),
            Size::new(side, side),
        )
    }
}

/// What stands of a merged region: waiting on what it merges, its meshes `M` being made, or
/// standing.
enum Merged<W, M> {
    Waiting(W),
    Merging(Making<Vec<M>>),
    Standing(Vec<Entity>),
}

impl<W, M> Merged<W, M> {
    /// Stands what is made, each by `spawn`, once it is: whether all of it stands with its meshes
    /// on the GPU, none while waiting.
    fn rise(
        &mut self,
        world: &mut World,
        mut spawn: impl FnMut(&mut World, M) -> Option<Entity>,
    ) -> Option<bool> {
        if let Merged::Merging(making) = self {
            let Some(made) = making.made() else {
                return Some(false);
            };
            let made = std::mem::take(made);
            *self = Merged::Standing(
                made.into_iter()
                    .filter_map(|next| spawn(world, next))
                    .collect(),
            );
        }
        let Merged::Standing(stood) = self else {
            return None;
        };
        Some(stood.iter().all(|&mesh| upload::sent(world, mesh)))
    }

    /// What stands of the region.
    fn stood(&self) -> &[Entity] {
        match self {
            Merged::Standing(stood) => stood,
            Merged::Waiting(_) | Merged::Merging(_) => &[],
        }
    }
}

/// What a task makes for the scenery, kept once it is made until a frame has room to stand it.
enum Making<T> {
    Task(Task<T>),
    Made(T),
}

impl<T> Making<T> {
    /// What the task made, once it has.
    fn made(&mut self) -> Option<&mut T> {
        if let Making::Task(task) = self
            && let Some(made) = check_ready(task)
        {
            *self = Making::Made(made);
        }
        match self {
            Making::Made(made) => Some(made),
            Making::Task(_) => None,
        }
    }
}

fn distance_to(bounds: Rect<Meters>, at: Pos<Meters>) -> f32 {
    let dx = (bounds.min().x - at.x).max(at.x - bounds.max().x).max(0.0);
    let dy = (bounds.min().y - at.y).max(at.y - bounds.max().y).max(0.0);
    (dx * dx + dy * dy).sqrt()
}

/// Stands the lights `plan` stands, and takes down those it no longer keeps.
fn stream_lights(world: &mut World, streaming: &mut Streaming, plan: &Plan, root: Entity) {
    let lights = &plan.lights;
    let gone: Vec<usize> = streaming
        .stood
        .lights
        .keys()
        .filter(|index| !lights.kept.contains(*index))
        .copied()
        .collect();
    for index in gone {
        if let Some(entity) = streaming.stood.lights.remove(&index) {
            world.despawn(entity);
        }
    }
    let near: Vec<usize> = lights
        .near
        .iter()
        .filter(|index| !streaming.stood.lights.contains_key(*index))
        .copied()
        .collect();
    let mut rng = Rng::fork_of(world);
    for index in near {
        let entity = spawn_light(
            world,
            &streaming.heights,
            &streaming.area.lights[index],
            &mut rng,
            root,
        );
        streaming.stood.lights.insert(index, entity);
    }
}

/// Takes `scenery` out of the view at once, and out of the world a few parts a frame, so a
/// whole area going doesn't hold up a frame. What its streaming held, which may be the last of
/// a whole area, is let go of off the frame.
pub fn despawn(world: &mut World, scenery: Scenery) {
    if let Some(streaming) = world.resource_mut::<Streams>().0.remove(&scenery.root) {
        if streaming.renderer == WorldRenderer::Gpu {
            instances::drop_arena(world, scenery.root);
        }
        AsyncComputeTaskPool::get()
            .spawn(async move { drop(streaming) })
            .detach();
    }
    take_away(world, scenery.root);
}

/// Takes `entity` out of the view at once, and out of the world with everything under it a few
/// parts a frame, so much going at once doesn't hold up a frame.
fn take_away(world: &mut World, entity: Entity) {
    let Ok(mut gone) = world.get_entity_mut(entity) else {
        return;
    };
    gone.insert(Visibility::Hidden);
    world.resource_mut::<TakenDown>().0.push(entity);
}

/// What is out of the view, still being taken out of the world.
#[derive(Resource, Default)]
struct TakenDown(Vec<Entity>);

fn take_down(world: &mut World) {
    let mut left = PARTS_TAKEN_DOWN_PER_FRAME;
    while left > 0
        && let Some(&root) = world.resource::<TakenDown>().0.last()
    {
        let part = world
            .get::<Children>(root)
            .and_then(|children| children.last().copied());
        let Some(part) = part else {
            world.resource_mut::<TakenDown>().0.pop();
            if let Ok(root) = world.get_entity_mut(root) {
                root.despawn();
            }
            continue;
        };
        left = left.saturating_sub(parts_of(world, part));
        world.despawn(part);
    }
}

/// How many entities `entity` is, with everything under it.
fn parts_of(world: &World, entity: Entity) -> usize {
    1 + world.get::<Children>(entity).map_or(0, |children| {
        children.iter().map(|child| parts_of(world, child)).sum()
    })
}

/// Lights `area` for the hour under its weather: its sun turned and coloured, or by night the full
/// moon in its place, the eye opened to how strongly it shines, its clouds carried
/// on by the wind, and the ambient light, the haze of the sun as it shines and the area's sky put
/// on `camera`; cheap enough to do every frame. Only what comes out different is written, so
/// what follows the light sees a change only when there is one.
pub fn light(
    world: &mut World,
    area: &Area,
    weather: &Weather,
    daylight: Daylight,
    scenery: Scenery,
    camera: Entity,
) {
    // Copied out rather than taken out by `resource_scope`, whose putting back counts as a
    // change to the look and has everything that follows it remade every frame.
    let look = *world.resource::<Look>();
    let look = &look;
    let (clouds, water) = (weather.clouds, weather.water);
    let sun = sun_at(area, weather, daylight, look);
    let elevation = sun.map(|sun| sun.elevation);
    let ambient = AmbientLight {
        color: area.lighting.ambient.color(),
        brightness: look.light.ambient_nits
            * sky::ambient_at(daylight, elevation, clouds, water, look),
        ..default()
    };
    world.resource_mut::<CloudSheet>().cover(clouds, water);
    let shining = sun.map(|sun| sky::shining(sun, clouds, water, look));
    if let (Some(sun), Some(shining), Some(entity)) = (sun, shining, scenery.sun) {
        if let Some(mut light) = world.get_mut::<DirectionalLight>(entity)
            && (light.color, light.illuminance) != (shining.color, shining.illuminance)
        {
            light.color = shining.color;
            light.illuminance = shining.illuminance;
        }
        let now = world.resource::<Time>().elapsed_secs();
        let drifted = world.resource_mut::<CloudDrift>().blown(
            shown_weather(weather, 0.0).wind,
            now,
            &look.clouds,
        );
        let center = area.size.bounds().center();
        let placed = lights::sun_transform(
            shining.elevation,
            shining.azimuth,
            Vec2::new(center.x, center.y),
        );
        let handoff = Handoff::of(world);
        let cast = world.resource_scope(|world, mut sheet: Mut<CloudSheet>| {
            sheet.lay(
                placed,
                drifted,
                &mut world.resource_mut::<Assets<Image>>(),
                handoff,
            )
        });
        if let Some(mut transform) = world.get_mut::<Transform>(entity) {
            transform.set_if_neq(cast);
        }
        if scenery.sky.is_some() || scenery.rain.is_some() {
            let heavens = Heavens::new(
                placed.back().as_vec3(),
                shining.color,
                shining.illuminance / (look.light.sun_lux * sky::clouded(clouds, water, look)),
                -lights::toward(sun.elevation, sun.azimuth),
                sky::night(daylight, sun, &look.sky),
                &look.clouds,
            )
            .covered(
                SkyWeather {
                    clouds,
                    water,
                    rain: weather.rain,
                    rain_lean: area.climate.rain.as_ref().map_or(Vec2::ZERO, |rainfall| {
                        rainfall.lean(shown_weather(weather, 0.0).wind)
                    }),
                },
                drifted,
                &look.clouds,
            );
            if let Some(dome) = scenery.sky {
                light_dome(world, dome, heavens);
            }
            if let Some(rain) = scenery.rain {
                light_rain(world, rain, heavens);
            }
        }
    }
    let mut tone = area.lighting.grade.tone();
    let mut lit = 1.0;
    if let (Some(shining), Some(noon)) = (shining, sun_at(area, weather, Daylight::Fixed, look)) {
        let opened = sky::adaptation(daylight, shining, look);
        tone.exposure.0 += opened.0;
        let sky_light = sky::ambient_at(daylight, elevation, clouds, water, look)
            / sky::ambient_at(Daylight::Fixed, None, clouds, water, look);
        lit = sky::lit(shining, sky_light, opened, noon, look);
    }
    let rains = world.get_resource::<CameraRig>().map_or(0.0, |rig| {
        world.resource::<RainField>().rain_at(rig.target().xz())
    });
    let moist = humidity(weather, look);
    let eye = world
        .get::<GlobalTransform>(camera)
        .map(GlobalTransform::translation);
    if let Some(eye) = eye {
        let ground = area.heights.height_at(Pos::new(eye.x, eye.z));
        if let Some(air) = scenery.air {
            let (volume, placed) = air_round(area, eye.xz(), ground, moist);
            hold_by(world, air, volume, same_air);
            hold(world, air, placed);
        }
        let color = sky::haze(sun, lit, area.lighting.sky, moist, rains, &look.sky);
        let mut banks: Vec<&FogBank> = area.fogs.iter().collect();
        banks.sort_by(|a, b| a.distance(eye.xz()).total_cmp(&b.distance(eye.xz())));
        let haze = Haze::new(
            color,
            ground.0,
            banks.into_iter().map(|bank| bank.seen(moist, rains)),
        );
        hold(world, camera, haze);
    }
    hold_by(world, camera, ambient, same_ambient);
    if let Some(grade) = world.get::<WorldPicture>(camera).map(|picture| picture.0) {
        hold(world, camera, camera::grading(tone, grade));
    }
    sky::show(world, camera, area.lighting.sky);
}

/// Gives `entity` `value`, unless it holds it already.
fn hold<C: Component<Mutability = Mutable> + PartialEq>(
    world: &mut World,
    entity: Entity,
    value: C,
) {
    hold_by(world, entity, value, C::eq);
}

/// Gives `entity` `value`, unless it holds one `same` takes for it already.
fn hold_by<C: Component<Mutability = Mutable>>(
    world: &mut World,
    entity: Entity,
    value: C,
    same: fn(&C, &C) -> bool,
) {
    let Ok(mut held) = world.get_entity_mut(entity) else {
        return;
    };
    match held.get_mut::<C>() {
        Some(mut current) => {
            if !same(&current, &value) {
                *current = value;
            }
        }
        None => {
            held.insert(value);
        }
    }
}

fn same_ambient(a: &AmbientLight, b: &AmbientLight) -> bool {
    (a.color, a.brightness, a.affects_lightmapped_meshes)
        == (b.color, b.brightness, b.affects_lightmapped_meshes)
}

fn same_air(a: &FogVolume, b: &FogVolume) -> bool {
    (
        a.fog_color,
        a.density_factor,
        &a.density_texture,
        a.density_texture_offset,
        a.absorption,
        a.scattering,
        a.scattering_asymmetry,
        a.light_tint,
        a.light_intensity,
    ) == (
        b.fog_color,
        b.density_factor,
        &b.density_texture,
        b.density_texture_offset,
        b.absorption,
        b.scattering,
        b.scattering_asymmetry,
        b.light_tint,
        b.light_intensity,
    )
}

/// The low air the sun's shafts show in round `eye` (x east, y south), over ground `ground` high:
/// in the bank of fog the eye stands in, as far round as its haze stays clear, as moist as
/// `moist`; none outside every bank.
fn air_round(area: &Area, eye: Vec2, ground: Meters, moist: f32) -> (FogVolume, Transform) {
    let at = Pos::new(eye.x, eye.y);
    area.fogs
        .iter()
        .find(|bank| bank.rect.contains(at))
        .and_then(|bank| {
            let reach = bank.haze.clear.0.max(1.0);
            let round = Rect::new(
                Pos::new(eye.x - reach, eye.y - reach),
                Size::new(reach * 2.0, reach * 2.0),
            );
            bank.rect
                .intersection(&round)
                .map(|within| sky::air(within, ground, bank.height, bank.haze.air(moist)))
        })
        .unwrap_or_else(|| sky::air(Rect::zero(), ground, Meters(0.0), 0.0))
}

/// How moist the air is under `weather`.
fn humidity(weather: &Weather, look: &Look) -> f32 {
    sky::humidity(weather.clouds, weather.water, &look.sky)
}

/// The dome the area's sky is painted on, its shader filled with the look's parameters.
fn spawn_dome(
    world: &mut World,
    content: &Content,
    look: &DomeLook,
    root: Entity,
) -> Option<Entity> {
    let shader = vfx_render::compiled_shader(world, look.shader)?;
    let params = ParamsUniform::pack(look.shader.get(content), |name| {
        look.params.get(name).copied()
    });
    let material = SkyMaterial {
        site: SiteUniform::default(),
        params,
        weather: world.resource::<WeatherImage>().0.clone(),
        heavens: Heavens::clear(world.resource::<CloudSheet>().look()),
        clouds: world.resource::<CloudSheet>().field(),
        shader,
    };
    let material = world.resource_mut::<Assets<SkyMaterial>>().add(material);
    let dome = sky_vfx::dome(&mut world.resource_mut::<Assets<Mesh>>(), material);
    Some(world.spawn((dome, ChildOf(root))).id())
}

/// Tells the sky on `dome` where its lights stand, how deep the night is and how the field of cloud
/// lies.
fn light_dome(world: &mut World, dome: Entity, heavens: Heavens) {
    let Some(handle) = world
        .get::<MeshMaterial3d<SkyMaterial>>(dome)
        .map(|material| material.0.clone())
    else {
        return;
    };
    let mut materials = world.resource_mut::<Assets<SkyMaterial>>();
    if materials
        .get(&handle)
        .is_some_and(|sky| sky.heavens != heavens)
        && let Some(mut sky) = materials.get_mut(&handle)
    {
        sky.heavens = heavens;
    }
}

/// The shells the area's rain falls on, its shader filled with the rainfall's parameters, hidden
/// until rain falls round the eye.
fn spawn_rain(
    world: &mut World,
    content: &Content,
    rainfall: &Rainfall,
    root: Entity,
) -> Option<Entity> {
    let shader = vfx_render::compiled_shader(world, rainfall.shader)?;
    let params = ParamsUniform::pack(rainfall.shader.get(content), |name| {
        rainfall.params.get(name).copied()
    });
    let material = RainMaterial {
        site: SiteUniform::default(),
        params,
        weather: world.resource::<WeatherImage>().0.clone(),
        heavens: Heavens::clear(world.resource::<CloudSheet>().look()),
        clouds: world.resource::<CloudSheet>().field(),
        shader,
    };
    let material = world.resource_mut::<Assets<RainMaterial>>().add(material);
    let shells = rain_vfx::shells(
        &mut world.resource_mut::<Assets<Mesh>>(),
        &rainfall.shells,
        material,
    );
    Some(
        world
            .spawn((shells, Visibility::Hidden, ChildOf(root)))
            .id(),
    )
}

/// Tells the rain on `shells` where the lights stand and how the field of cloud lies, as the sky
/// is told.
fn light_rain(world: &mut World, shells: Entity, heavens: Heavens) {
    let Some(handle) = world
        .get::<MeshMaterial3d<RainMaterial>>(shells)
        .map(|material| material.0.clone())
    else {
        return;
    };
    let mut materials = world.resource_mut::<Assets<RainMaterial>>();
    if materials
        .get(&handle)
        .is_some_and(|rain| rain.heavens != heavens)
        && let Some(mut rain) = materials.get_mut(&handle)
    {
        rain.heavens = heavens;
    }
}

fn sun_at(area: &Area, weather: &Weather, daylight: Daylight, look: &Look) -> Option<SunLight> {
    area.lighting.sun.map(|sun| {
        sky::sun_at(
            daylight,
            sun.elevation,
            sun.azimuth,
            sun.color.color(),
            weather.clouds,
            weather.water,
            look,
        )
    })
}

/// The wind, cloud and rain `weather` makes for what the client draws, lightning lighting the
/// world `flash` of the way to a strike close by.
pub fn shown_weather(weather: &Weather, flash: f32) -> ShownWeather {
    let (sin, cos) = weather.wind.direction.radians().0.sin_cos();
    ShownWeather {
        wind: Vec2::new(sin, -cos) * weather.wind.strength,
        clouds: weather.clouds,
        water: weather.water,
        rain: weather.rain,
        flash,
    }
}

pub fn grade(lighting: &Lighting) -> AreaGrade {
    let color = lighting.grade.color.color().to_linear();
    AreaGrade {
        color: Vec3::new(color.red, color.green, color.blue),
        strength: lighting.grade.strength,
    }
}

/// Lays `scenery`'s ground anew on `heights` painted with `paint`, as a stroke shapes it.
pub fn lay_ground(world: &mut World, scenery: Scenery, heights: &Heightfield, paint: &PaintMap) {
    world.resource_scope(|world, mut streams: Mut<Streams>| {
        if let Some(streaming) = streams.0.get_mut(&scenery.root) {
            lay_streamed_ground(world, streaming, heights, paint);
        }
    });
}

fn lay_streamed_ground(
    world: &mut World,
    streaming: &mut Streaming,
    heights: &Heightfield,
    paint: &PaintMap,
) {
    streaming.heights = heights.clone();
    streaming.paint = paint.clone();
    if let Some(mut atlas) = world.get_mut::<TerrainAtlas>(streaming.terrain) {
        atlas.lay(heights, paint, &streaming.water);
    }
    streaming.covers.ground_changed();
    streaming.whole = false;
}

/// Stands the terrain of `area` under `root`: its ground drawn in the terrains laid under it, the
/// pictures white until they load, from an atlas holding the area's ground.
fn spawn_terrain(
    world: &mut World,
    content: &Content,
    area: &Area,
    look: &Look,
    root: Entity,
) -> Entity {
    let layers = ground_layers(content, area);
    let weather = world.resource::<WeatherImage>().0.clone();
    let (paint, pictures) = world.resource_scope(|world, mut images: Mut<Assets<Image>>| {
        let assets = world.resource::<AssetServer>();
        terrain::ground_paint(&layers, &look.materials, weather, assets, &mut images)
    });
    let mut atlas = TerrainAtlas::default();
    atlas.lay(&area.heights, &area.paint, &area.water);
    world
        .spawn((
            GroundDraw {
                patches: Arc::new([]),
                paint,
                sheets: Arc::new([]),
            },
            atlas,
            pictures,
            Transform::default(),
            Visibility::default(),
            ChildOf(root),
        ))
        .id()
}

/// Each of the area's terrains as the ground is painted with it.
fn ground_layers(content: &Content, area: &Area) -> Vec<GroundLayer> {
    area.terrains
        .iter()
        .map(|terrain| {
            let def = terrain.get(content);
            GroundLayer {
                color: def.color.color().to_linear(),
                picture: def
                    .picture
                    .as_ref()
                    .map(|picture| (picture.image.path(), picture.tile)),
                wet: def.wet,
            }
        })
        .collect()
}

fn spawn_light(
    world: &mut World,
    heights: &Heightfield,
    light: &PointLightDef,
    rng: &mut Rng,
    root: Entity,
) -> Entity {
    let ground = heights.height_at(light.at);
    let point = lights::point(light.color.color(), light.intensity, light.radius.0);
    let full = point.intensity;
    let mut spawned = world.spawn((
        point,
        Transform::from_translation(world_point(light.at, ground + light.height)),
        ChildOf(root),
    ));
    if light.flicker > 0.0 {
        spawned.insert(Flicker {
            amount: light.flicker,
            full,
            phase: rng.rand_float() * std::f32::consts::TAU,
        });
    }
    spawned.id()
}
