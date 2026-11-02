//! What stands in a world for an area: its ground as coloured meshes of its chunks, the covers
//! its terrains grow, its props on the ground, its point lights, its sun, the sky its sun lights
//! and the air over it, every one a child of one root. The client stands the area the player is
//! in; the editor stands the area it edits, on render layers of its own, and both light it alike
//! for the hour. The ground stands whole, finer the nearer the view, and the covers, props and
//! lights stand within the view's reach alone, a few a frame through `SceneryPlugin` as the view
//! moves, the props near the eye as scenes and the rest merged, so an area costs what the view
//! takes in, however far it stretches.

mod covers;
mod props;
mod regions;

pub use props::prop_transform;

use covers::Covers;
use props::Props;
use regions::Regions;

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;
use std::sync::Arc;

use bevy::app::Propagate;
use bevy::camera::visibility::RenderLayers;
use bevy::light::atmosphere::ScatteringMedium;
use bevy::mesh::MeshTag;
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, IoTaskPool, Task};
use rayon::prelude::*;

use super::lighting::DomeLook;
use super::prop::PropId;
use super::terrain::PieceShape;
use super::{Area, Lighting, PointLightDef};
use crate::core::content::{Content, FileHash, FileHashes};
use crate::core::ground::{Heightfield, OVERVIEW_STRIDE, PaintMap, SAMPLES_PER_METER, TileAt};
use crate::core::math::{Pos, Rect, Rng, Size};
use crate::core::render::camera::{View, WorldPicture};
use crate::core::render::present::AreaGrade;
use crate::core::render::world::clouds::{self, CloudSheet};
use crate::core::render::world::lights::{self, Flicker};
use crate::core::render::world::sky::{self, Daylight, SunLight};
use crate::core::render::world::terrain::{
    self, ChunkPaint, ChunkPaints, GroundLayer, GroundMaterial,
};
use crate::core::render::{Look, camera, world_point};
use crate::core::spatial::SpatialIndex;
use crate::core::units::{MeterSize, Meters};
use crate::core::vfx::cover::Climate;
use crate::core::vfx::render as vfx_render;
use crate::core::vfx::sky::{self as sky_vfx, Heavens, SkyMaterial};
use crate::core::vfx::uniform::{ParamsUniform, SiteUniform, WeatherImage};

/// An area's scenery as it stands in a world: everything hangs under `root`, the terrain mesh is
/// `terrain`, `sun` is the directional light when the area has one, the sun by day and the full
/// moon by night, and `sky` the dome its sky is painted on when the area paints one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scenery {
    pub root: Entity,
    pub terrain: Entity,
    pub sun: Option<Entity>,
    pub sky: Option<Entity>,
}

/// Stands scenery as the view moves, a few pieces a frame, so the frames keep coming however much
/// the view takes in at once.
pub struct SceneryPlugin;

impl Plugin for SceneryPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, stream)
            .add_observer(props::level_ready_scene);
    }
}

/// Stands `area` in the world on `layers`: its sun, sky and air at once, and its ground, covers,
/// props and lights as the view comes to them, from where [`view_from`] says it is.
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
    let look = world.resource::<Look>().clone();
    let humidity = humidity(area, &look);
    let atmosphere = sky::atmosphere(
        &mut world.resource_mut::<Assets<ScatteringMedium>>(),
        area.lighting.sky,
        humidity,
        &look.sky,
    );
    world.spawn((atmosphere, ChildOf(root)));
    let mut dome = None;
    if area.lighting.sky.is_air() {
        world.spawn((
            sky::air(area.size.bounds(), humidity, &look.sky),
            ChildOf(root),
        ));
        dome = area
            .lighting
            .dome
            .as_ref()
            .and_then(|look| spawn_dome(world, content, look, root));
    }
    let (terrain, paints) = spawn_terrain(world, content, area, &look, root);
    let props = Props::new(content, area);
    let covers = Covers::new(world, content, area);
    let mut lights = SpatialIndex::new(PLACEMENT_CELL);
    lights.refile_spots(area.lights.iter().map(|light| light.at));
    world.entity_mut(root).insert(Streaming {
        content: content.clone(),
        area: area.clone(),
        heights: area.heights.clone(),
        paint: area.paint.clone(),
        terrain,
        paints,
        regions: Regions::of(area),
        covers,
        props,
        lights,
        view: None,
        stood: Stood::default(),
        loading: HashMap::default(),
        making: HashMap::default(),
        settled: false,
        stood_for: None,
    });
    Scenery {
        root,
        terrain,
        sun,
        sky: dome,
    }
}

/// Tells the scenery where `view` looks at, and how far around it the covers, props and lights
/// are to stand: a cover's pieces and a prop only where they span a pixel of the view, a prop's
/// meshes no finer than the view shows them. The ground stands whole, finer the nearer the view.
/// What stands follows over the frames.
pub fn view_from(world: &mut World, scenery: Scenery, at: Pos<Meters>, reach: Meters, view: View) {
    if let Some(mut streaming) = world.get_mut::<Streaming>(scenery.root) {
        streaming.view = Some(Seen { at, reach, view });
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
    covers
        && sky
        && props_of(area).iter().all(|prop| {
            prop.get(content).materials.values().all(|material| {
                vfx_render::compiled_shader(world, material.get(content).shader).is_some()
            })
        })
}

/// Whether any scenery has yet to stand what its view takes in.
pub fn standing(world: &mut World) -> bool {
    world
        .query::<&Streaming>()
        .iter(world)
        .any(|streaming| !streaming.settled)
}

/// Whether `scenery` stands all its view takes in.
pub fn stood(world: &World, scenery: Scenery) -> bool {
    world
        .get::<Streaming>(scenery.root)
        .is_some_and(|streaming| streaming.settled)
}

/// How many sceneries stand.
pub fn sceneries(world: &mut World) -> usize {
    world.query::<&Streaming>().iter(world).count()
}

/// What an area's scenery stands as the view moves: the area and what it's drawn from, where the
/// view is, and what stands now, by what it stands for.
#[derive(Component)]
struct Streaming {
    content: Content,
    area: Arc<Area>,
    heights: Heightfield,
    paint: PaintMap,
    terrain: Entity,
    paints: ChunkPaints,
    regions: Regions,
    covers: Covers,
    props: Props,
    lights: SpatialIndex,
    view: Option<Seen>,
    stood: Stood,
    /// The tiles of ground being loaded, for the near chunks waiting on them.
    loading: HashMap<TileAt, Task<()>>,
    /// The chunks of ground being made, each its mesh and the paint under it.
    making: HashMap<Node, Making<(Mesh, ChunkPaint)>>,
    settled: bool,
    /// The view all that it takes in stands for, once it does, so a view that stays put costs
    /// nothing to stream.
    stood_for: Option<Seen>,
}

/// Where the view looks at, how far around it the scenery stands, and the view.
#[derive(Clone, Copy, PartialEq)]
struct Seen {
    at: Pos<Meters>,
    reach: Meters,
    view: View,
}

#[derive(Default)]
struct Stood {
    ground: HashMap<Node, Entity>,
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

/// The samples along a leaf chunk's side, and how near the view a node must lie, in its own
/// sides, to be cut into finer ones.
const LEAF_SAMPLES: u32 = 128;
/// The pixels along every chunk's paint: a pixel per stride of its samples, and one beyond each
/// edge.
const PAINT_SIDE: u32 = LEAF_SAMPLES + 2;
const SPLIT_WITHIN: f32 = 1.5;
/// The level from which a chunk's stride is the overview's, so it stands from the overview alone;
/// a finer chunk waits for the tiles of ground under it.
const OVERVIEW_LEVEL: u32 = OVERVIEW_STRIDE.trailing_zeros();
/// How placements are filed to be found near the view, and the side of the leaves of the
/// quadtree the props stand over.
const PLACEMENT_CELL: Meters = Meters(32.0);
/// How far past its reach the view keeps what stands, so pacing back and forth over the edge
/// stands nothing twice.
const KEEP_SHARE: f32 = 1.25;
/// How many bytes of new meshes a frame sends to the GPU: what stands past them waits for the
/// next frame, so a view taking in much at once costs many frames a little rather than one frame a
/// lot. A piece larger than this stands in a frame of its own.
const MESH_BYTES_PER_FRAME: usize = 2 << 20;
/// How much stands in one frame, and among the props how many whose model no earlier prop asked
/// for: a model's load falls on the frames after the prop that starts it.
const COVERS_PER_FRAME: usize = 16;
const PROPS_PER_FRAME: usize = 16;
const FRESH_MODELS_PER_FRAME: usize = 1;
const MERGES_PER_FRAME: usize = 4;

fn stream(world: &mut World) {
    let roots: Vec<Entity> = world
        .query_filtered::<Entity, With<Streaming>>()
        .iter(world)
        .collect();
    for root in roots {
        let Some(mut streaming) = world.entity_mut(root).take::<Streaming>() else {
            continue;
        };
        let Some(seen) = streaming.view else {
            world.entity_mut(root).insert(streaming);
            continue;
        };
        if streaming.stood_for == Some(seen) && streaming.loading.is_empty() {
            world.entity_mut(root).insert(streaming);
            continue;
        }
        let Seen { at, reach, .. } = seen;
        let keep = Meters(reach.0 * KEEP_SHARE);
        let mut budget = Budget::new();
        stream_ground(world, &mut streaming, at, &mut budget);
        let cut = streaming.regions.cut(&streaming.heights, seen);
        let props_stand = props::stand(world, &mut streaming, root, seen, &cut, &mut budget);
        let covers_stand = covers::stand(world, &mut streaming, root, seen, &cut, &mut budget);
        if props_stand && covers_stand {
            props::clear(world, &mut streaming, seen, &cut);
            covers::clear(world, &mut streaming, seen, &cut);
        } else {
            budget.defer();
        }
        props::level_props(world, &streaming, &seen.view);
        stream_lights(world, &mut streaming, root, at, reach, keep);
        streaming.settled = !budget.spent;
        streaming.stood_for = streaming.settled.then_some(seen);
        world.entity_mut(root).insert(streaming);
    }
}

/// What a frame may still stand.
struct Budget {
    mesh_bytes: usize,
    covers: usize,
    props: usize,
    fresh: usize,
    merges: usize,
    /// Whether the frame left something to stand.
    spent: bool,
}

impl Budget {
    fn new() -> Budget {
        Budget {
            mesh_bytes: MESH_BYTES_PER_FRAME,
            covers: COVERS_PER_FRAME,
            props: PROPS_PER_FRAME,
            fresh: FRESH_MODELS_PER_FRAME,
            merges: MERGES_PER_FRAME,
            spent: false,
        }
    }

    /// Leaves something to stand on a later frame, as it waits on what is being made or loaded.
    fn defer(&mut self) {
        self.spent = true;
    }

    /// Whether the frame sends meshes of `bytes` to the GPU: when it has room for them, or has sent
    /// none yet.
    fn send(&mut self, bytes: usize) -> bool {
        if bytes > self.mesh_bytes && self.mesh_bytes < MESH_BYTES_PER_FRAME {
            self.spent = true;
            return false;
        }
        self.mesh_bytes = self.mesh_bytes.saturating_sub(bytes);
        true
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

fn stream_ground(
    world: &mut World,
    streaming: &mut Streaming,
    at: Pos<Meters>,
    budget: &mut Budget,
) {
    streaming
        .loading
        .retain(|_, task| check_ready(task).is_none());
    let wanted: HashSet<Node> = nodes_for(&streaming.heights, at).into_iter().collect();
    streaming.making.retain(|node, _| wanted.contains(node));
    let gone: Vec<Node> = streaming
        .stood
        .ground
        .keys()
        .filter(|&node| !wanted.contains(node))
        .copied()
        .collect();
    let mut missing: Vec<Node> = wanted
        .iter()
        .filter(|&node| !streaming.stood.ground.contains_key(node))
        .copied()
        .collect();
    if missing.is_empty() {
        for node in gone {
            if let Some(entity) = streaming.stood.ground.remove(&node) {
                take_down_chunk(world, &mut streaming.paints, entity);
            }
        }
        return;
    }
    missing.sort_by_key(|node| node.level);
    for node in missing {
        if !ground_under(streaming, node) {
            budget.defer();
            continue;
        }
        let making = streaming.making.entry(node).or_insert_with(|| {
            Making::Task(make_chunk(&streaming.heights, &streaming.paint, node))
        });
        let Some((mesh, _)) = making.made() else {
            budget.defer();
            continue;
        };
        if !budget.send(mesh_bytes(mesh)) {
            break;
        }
        let Some((mesh, paint)) = streaming.making.remove(&node).and_then(Making::into_made) else {
            continue;
        };
        let entity = spawn_chunk(world, streaming, mesh, &paint);
        streaming.stood.ground.insert(node, entity);
    }
    if wanted
        .iter()
        .all(|node| streaming.stood.ground.contains_key(node))
    {
        for node in gone {
            if let Some(entity) = streaming.stood.ground.remove(&node) {
                take_down_chunk(world, &mut streaming.paints, entity);
            }
        }
    }
}

/// Whether the ground under `node` is there to stand it on: the overview, for a chunk as coarse as
/// it; else every tile under it loaded, each not yet being loaded set loading on a task.
fn ground_under(streaming: &mut Streaming, node: Node) -> bool {
    if node.level >= OVERVIEW_LEVEL {
        return true;
    }
    let (heights, paint) = (&streaming.heights, &streaming.paint);
    let rect = node_rect(node);
    let side = rect.side as f32 / SAMPLES_PER_METER as f32;
    let bounds = Rect::new(
        Heightfield::sample_pos(rect.first.0, rect.first.1),
        Size::new(side, side),
    );
    let mut ready = true;
    for tile in heights.size().tiles_within(bounds) {
        if heights.is_loaded(tile) && paint.is_loaded(tile) {
            continue;
        }
        ready = false;
        if streaming.loading.contains_key(&tile) {
            continue;
        }
        let (heights, paint) = (heights.clone(), paint.clone());
        let task = IoTaskPool::get().spawn(async move {
            for problem in [heights.load(tile).err(), paint.load(tile).err()]
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

/// The nodes of the quadtree over the ground's samples that stand for the view at `at`: every node
/// is cut in four while it lies within `SPLIT_WITHIN` of its sides from the view and is wider than
/// a leaf; what remains covers the ground once over, finer the nearer the view.
fn nodes_for(heights: &Heightfield, at: Pos<Meters>) -> Vec<Node> {
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
        let side = (rect.side as f32) / SAMPLES_PER_METER as f32;
        let bounds = Rect::new(
            Heightfield::sample_pos(rect.first.0, rect.first.1),
            Size::new(side, side),
        );
        let near = distance_to(bounds, at) < side * SPLIT_WITHIN;
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

/// What a task makes for the scenery, kept once it is made until a frame has room to stand it.
enum Making<T> {
    Task(Task<T>),
    Made(T),
}

impl<T> Making<T> {
    /// What the task made, once it has.
    fn made(&mut self) -> Option<&T> {
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

    fn into_made(self) -> Option<T> {
        match self {
            Making::Made(made) => Some(made),
            Making::Task(_) => None,
        }
    }
}

/// The bytes `mesh` takes on the GPU.
fn mesh_bytes(mesh: &Mesh) -> usize {
    mesh.get_vertex_buffer_size() + mesh.get_index_buffer_bytes().map_or(0, <[u8]>::len)
}

/// Makes the chunk of ground `node` on a task: its mesh and the paint under it.
fn make_chunk(heights: &Heightfield, paint: &PaintMap, node: Node) -> Task<(Mesh, ChunkPaint)> {
    let (heights, paint) = (heights.clone(), paint.clone());
    AsyncComputeTaskPool::get().spawn(async move { made_chunk(&heights, &paint, node) })
}

fn made_chunk(heights: &Heightfield, paint: &PaintMap, node: Node) -> (Mesh, ChunkPaint) {
    (
        chunk_mesh(heights, node),
        ChunkPaint::of(paint, chunk_samples(node)),
    )
}

fn spawn_chunk(
    world: &mut World,
    streaming: &mut Streaming,
    mesh: Mesh,
    paint: &ChunkPaint,
) -> Entity {
    let mesh = world.resource_mut::<Assets<Mesh>>().add(mesh);
    let tag = streaming.paints.lay(world, paint);
    world
        .spawn((
            Mesh3d(mesh),
            MeshMaterial3d(streaming.paints.material().clone()),
            tag,
            Transform::default(),
            ChildOf(streaming.terrain),
        ))
        .id()
}

/// Takes down a chunk of ground, freeing the layer its paint lay on.
fn take_down_chunk(world: &mut World, paints: &mut ChunkPaints, chunk: Entity) {
    if let Some(tag) = world.get::<MeshTag>(chunk) {
        paints.lift(tag);
    }
    world.despawn(chunk);
}

fn chunk_samples(node: Node) -> terrain::ChunkSamples {
    let rect = node_rect(node);
    terrain::ChunkSamples {
        first: rect.first,
        side: rect.side,
        stride: 1 << node.level,
    }
}

fn chunk_mesh(heights: &Heightfield, node: Node) -> Mesh {
    let (columns, rows) = heights.samples();
    let rect = node_rect(node);
    let stride = 1 << node.level;
    terrain::chunk(
        heights,
        rect.first,
        rect.side.min(columns - 1 - rect.first.0),
        rect.side.min(rows - 1 - rect.first.1),
        stride,
    )
}

fn far(from: Pos<Meters>, to: Pos<Meters>, by: Meters) -> bool {
    (from - to).length() > by.0
}

fn distance_to(bounds: Rect<Meters>, at: Pos<Meters>) -> f32 {
    let dx = (bounds.min().x - at.x).max(at.x - bounds.max().x).max(0.0);
    let dy = (bounds.min().y - at.y).max(at.y - bounds.max().y).max(0.0);
    (dx * dx + dy * dy).sqrt()
}

fn stream_lights(
    world: &mut World,
    streaming: &mut Streaming,
    root: Entity,
    at: Pos<Meters>,
    reach: Meters,
    keep: Meters,
) {
    let area = streaming.area.clone();
    let gone: Vec<usize> = streaming
        .stood
        .lights
        .keys()
        .filter(|&&index| far(area.lights[index].at, at, keep))
        .copied()
        .collect();
    for index in gone {
        if let Some(entity) = streaming.stood.lights.remove(&index) {
            world.despawn(entity);
        }
    }
    let near: Vec<usize> = streaming
        .lights
        .near(at, reach)
        .filter(|&index| {
            !streaming.stood.lights.contains_key(&index) && !far(area.lights[index].at, at, reach)
        })
        .collect();
    let mut rng = Rng::from_entropy();
    for index in near {
        let entity = spawn_light(
            world,
            &streaming.heights,
            &area.lights[index],
            &mut rng,
            root,
        );
        streaming.stood.lights.insert(index, entity);
    }
}

pub fn despawn(world: &mut World, scenery: Scenery) {
    if world.get_entity(scenery.root).is_ok() {
        world.despawn(scenery.root);
    }
}

/// Lights `area` for the hour under its weather: its sun turned and coloured, or by night the full
/// moon in its place, the eye opened to how strongly it shines, its clouds carried
/// on by the wind, and the ambient light, the haze of the sun as it shines and the area's sky put
/// on `camera`; cheap enough to do every frame.
pub fn light(world: &mut World, area: &Area, daylight: Daylight, scenery: Scenery, camera: Entity) {
    let look = world.resource::<Look>().clone();
    let (clouds, water) = (area.weather.clouds, area.weather.water);
    let sun = sun_at(area, daylight, &look);
    let elevation = sun.map(|sun| sun.elevation);
    let ambient = AmbientLight {
        color: area.lighting.ambient.color(),
        brightness: look.light.ambient_nits
            * sky::ambient_at(daylight, elevation, clouds, water, &look),
        ..default()
    };
    world.resource_mut::<CloudSheet>().cover(clouds, water);
    let shining = sun.map(|sun| sky::shining(sun, clouds, water, &look));
    if let (Some(sun), Some(shining), Some(entity)) = (sun, shining, scenery.sun) {
        if let Some(mut light) = world.get_mut::<DirectionalLight>(entity) {
            light.color = shining.color;
            light.illuminance = shining.illuminance;
        }
        let drifted = clouds::drift(
            climate(area).wind,
            world.resource::<Time>().elapsed_secs(),
            &look.clouds,
        );
        let center = area.size.bounds().center();
        let placed = lights::sun_transform(
            shining.elevation,
            shining.azimuth,
            Vec2::new(center.x, center.y),
        );
        let cast = world.resource_scope(|world, mut sheet: Mut<CloudSheet>| {
            sheet.lay(placed, drifted, &mut world.resource_mut::<Assets<Image>>())
        });
        if let Some(mut transform) = world.get_mut::<Transform>(entity) {
            *transform = cast;
        }
        if let Some(dome) = scenery.sky {
            let heavens = Heavens::new(
                placed.back().as_vec3(),
                shining.color,
                shining.illuminance / (look.light.sun_lux * sky::clouded(clouds, water, &look)),
                -lights::toward(sun.elevation, sun.azimuth),
                sky::night(daylight, sun, &look.sky),
                &look.clouds,
            )
            .covered(clouds, drifted, &look.clouds);
            light_dome(world, dome, heavens);
        }
    }
    let mut tone = area.lighting.grade.tone();
    let mut lit = 1.0;
    if let (Some(shining), Some(noon)) = (shining, sun_at(area, Daylight::Fixed, &look)) {
        let opened = sky::adaptation(daylight, shining, &look);
        tone.exposure.0 += opened.0;
        let sky_light = sky::ambient_at(daylight, elevation, clouds, water, &look)
            / sky::ambient_at(Daylight::Fixed, None, clouds, water, &look);
        lit = sky::lit(shining, sky_light, opened, noon, &look);
    }
    if let Ok(mut camera) = world.get_entity_mut(camera) {
        let grade = camera.get::<WorldPicture>().map(|picture| picture.0);
        let haze = sky::haze(
            sun,
            lit,
            area.lighting.sky,
            humidity(area, &look),
            &look.sky,
        );
        camera.insert((
            ambient,
            sky::Haze {
                fog: haze,
                air_height: look.sky.air_height,
                heights: area.heights.clone(),
            },
        ));
        if let Some(grade) = grade {
            camera.insert(camera::grading(tone, grade));
        }
    }
    sky::show(world, camera, area.lighting.sky);
}

/// How moist the area's air is under its weather.
fn humidity(area: &Area, look: &Look) -> f32 {
    sky::humidity(area.weather.clouds, area.weather.water, &look.sky)
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

fn sun_at(area: &Area, daylight: Daylight, look: &Look) -> Option<SunLight> {
    area.lighting.sun.map(|sun| {
        sky::sun_at(
            daylight,
            sun.elevation,
            sun.azimuth,
            sun.color.color(),
            area.weather.clouds,
            area.weather.water,
            look,
        )
    })
}

/// The wind and cloud the area's covers move under.
pub fn climate(area: &Area) -> Climate {
    let (sin, cos) = area.weather.wind.direction.radians().0.sin_cos();
    Climate {
        wind: Vec2::new(sin, -cos) * area.weather.wind.strength,
        clouds: area.weather.clouds,
        water: area.weather.water,
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
    let Some(mut streaming) = world.entity_mut(scenery.root).take::<Streaming>() else {
        return;
    };
    streaming.heights = heights.clone();
    streaming.paint = paint.clone();
    streaming.making.clear();
    let stood: Vec<(Node, Entity)> = streaming
        .stood
        .ground
        .iter()
        .map(|(&node, &entity)| (node, entity))
        .collect();
    let made: Vec<(Entity, Mesh, ChunkPaint)> = stood
        .into_par_iter()
        .map(|(node, entity)| {
            let (mesh, paint) = made_chunk(heights, paint, node);
            (entity, mesh, paint)
        })
        .collect();
    for (entity, mesh, paint) in made {
        let mesh = world.resource_mut::<Assets<Mesh>>().add(mesh);
        if let Ok(mut chunk) = world.get_entity_mut(entity) {
            chunk.insert(Mesh3d(mesh));
        }
        let Some(tag) = world.get::<MeshTag>(entity).cloned() else {
            continue;
        };
        streaming.paints.relay(world, &paint, &tag);
    }
    streaming.covers.ground_changed();
    world.entity_mut(scenery.root).insert(streaming);
}

/// The ground's root, holding the material its chunks are painted with, and their paint.
fn spawn_terrain(
    world: &mut World,
    content: &Content,
    area: &Area,
    look: &Look,
    root: Entity,
) -> (Entity, ChunkPaints) {
    let layers = ground_layers(content, area);
    let (material, pictures) = world.resource_scope(|world, mut images: Mut<Assets<Image>>| {
        let assets = world.resource::<AssetServer>();
        terrain::ground_material(&layers, &look.materials, assets, &mut images, PAINT_SIDE)
    });
    let material = world.resource_mut::<Assets<GroundMaterial>>().add(material);
    let terrain = world
        .spawn((
            MeshMaterial3d(material.clone()),
            pictures,
            Transform::default(),
            Visibility::default(),
            ChildOf(root),
        ))
        .id();
    (terrain, ChunkPaints::new(material, PAINT_SIDE))
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
