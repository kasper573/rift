//! What stands in a world for an area: its ground as coloured meshes of its chunks, the covers
//! its terrains grow, its props as scenes on the ground, its point lights, its sun, the sky its
//! sun lights and the air over it, every one a child of one root. The client stands the area the
//! player is in; the editor stands the area it edits, on render layers of its own, and both light
//! it alike for the hour. The ground stands whole, finer the nearer the view, and the covers,
//! props and lights stand within the view's reach alone, a few a frame through `SceneryPlugin`
//! as the view moves, so an area costs what the view takes in, however far it stretches.

use std::sync::Arc;

use bevy::app::Propagate;
use bevy::camera::primitives::MeshAabb;
use bevy::camera::visibility::RenderLayers;
use bevy::gltf::Gltf;
use bevy::light::NotShadowCaster;
use bevy::light::atmosphere::ScatteringMedium;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use super::lighting::DomeLook;
use super::prop::PropId;
use super::terrain::{CoverPiece, PieceShape};
use super::{Area, Lighting, PointLightDef, PropPlacement};
use crate::core::content::media::GltfInfos;
use crate::core::content::{Content, FileHash, FileHashes};
use crate::core::geometry::Radians;
use crate::core::geometry::{PlacedShape, Shape};
use crate::core::ground::{Heightfield, Paint, SAMPLES_PER_METER};
use crate::core::math::{Pos, Rect, Rng, Size};
use crate::core::pace::Pace;
use crate::core::render::present::AreaGrade;
use crate::core::render::world::clouds::{self, CloudSheet};
use crate::core::render::world::cover;
use crate::core::render::world::fade::Fades;
use crate::core::render::world::lights::{self, AMBIENT_NITS, Flicker, SUN_LUX};
use crate::core::render::world::material::{MaterialLook, MaterialLooks};
use crate::core::render::world::sky::{self, Daylight, SunLight};
use crate::core::render::world::terrain::{self, GroundLayer, GroundMaterial};
use crate::core::render::{camera, world_point};
use crate::core::spatial::SpatialIndex;
use crate::core::time::Seconds;
use crate::core::units::{MeterSize, Meters};
use crate::core::vfx::ParamFill;
use crate::core::vfx::cover::{self as cover_vfx, Climate, CoverMaterial};
use crate::core::vfx::render::{self as vfx_render, ParamsUniform, Run, SiteFrame, SiteUniform};
use crate::core::vfx::shader::Stage;
use crate::core::vfx::sky::{self as sky_vfx, Heavens, SkyMaterial};

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

pub struct SceneryPlugin;

impl Plugin for SceneryPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Pace>().add_systems(Update, stream);
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
    let humidity = humidity(area);
    let atmosphere = sky::atmosphere(
        &mut world.resource_mut::<Assets<ScatteringMedium>>(),
        area.lighting.sky,
        humidity,
    );
    world.spawn((atmosphere, ChildOf(root)));
    let mut dome = None;
    if area.lighting.sky.is_air() {
        world.spawn((sky::air(area.size.bounds(), humidity), ChildOf(root)));
        dome = area
            .lighting
            .dome
            .as_ref()
            .and_then(|look| spawn_dome(world, content, look, root));
    }
    let terrain = spawn_terrain(world, content, area, root);
    let covers = cover_layers(world, content, area);
    let mut props = SpatialIndex::new(PLACEMENT_CELL);
    props.refile(
        area.props
            .iter()
            .map(|placed| Rect::new(placed.at, Size::zero())),
    );
    let mut lights = SpatialIndex::new(PLACEMENT_CELL);
    lights.refile(
        area.lights
            .iter()
            .map(|light| Rect::new(light.at, Size::zero())),
    );
    world.entity_mut(root).insert(Streaming {
        content: content.clone(),
        area: area.clone(),
        heights: area.heights.clone(),
        paint: area.paint.clone(),
        terrain,
        covers,
        props,
        lights,
        view: None,
        stood: Stood::default(),
        settled: false,
    });
    Scenery {
        root,
        terrain,
        sun,
        sky: dome,
    }
}

/// Tells the scenery where the view is, and how far around it the covers, props and lights are to
/// stand; the ground stands whole, finer the nearer the view. What stands follows over the frames.
pub fn view_from(world: &mut World, scenery: Scenery, at: Pos<Meters>, reach: Meters) {
    if let Some(mut streaming) = world.get_mut::<Streaming>(scenery.root) {
        streaming.view = Some((at, reach));
    }
}

/// The files the scenery of an area is made from beyond the area itself, each with what it held
/// when read: its terrains, their covers' shaders, the props it stands and strews, and their
/// models. Scenery stood from equal sources looks alike, so it need not stand again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sources(Vec<(String, FileHash)>);

impl Sources {
    pub fn of(content: &Content, area: &Area) -> Sources {
        let mut files: Vec<&str> = Vec::new();
        let mut props: Vec<PropId> = area.props.iter().map(|placed| placed.prop).collect();
        for terrain in &area.terrains {
            files.push(terrain.path(content));
            let Some(cover) = &terrain.get(content).cover else {
                continue;
            };
            files.push(cover.shader.path(content));
            props.extend(cover.pieces.iter().filter_map(|piece| match piece.shape {
                PieceShape::Prop { prop } => Some(prop),
                PieceShape::Blade { .. } => None,
            }));
        }
        if let Some(dome) = &area.lighting.dome {
            files.push(dome.shader.path(content));
        }
        for prop in props {
            let def = prop.get(content);
            files.push(prop.path(content));
            files.push(def.model.as_str());
            for material in def.materials.values() {
                files.push(material.path(content));
                files.push(material.get(content).shader.path(content));
            }
        }
        let hashes = &content.built::<FileHashes>().0;
        Sources(
            hashes
                .iter()
                .filter(|(path, _)| {
                    files.iter().any(|file| {
                        path.strip_prefix(file)
                            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
                    })
                })
                .map(|(path, hash)| (path.clone(), hash.clone()))
                .collect(),
        )
    }
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
        && area.props.iter().all(|placed| {
            placed.prop.get(content).materials.values().all(|material| {
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

/// What an area's scenery stands as the view moves: the area and what it's drawn from, where the
/// view is, and what stands now, by what it stands for.
#[derive(Component)]
struct Streaming {
    content: Content,
    area: Arc<Area>,
    heights: Heightfield,
    paint: Paint,
    terrain: Entity,
    covers: Vec<CoverLayer>,
    props: SpatialIndex,
    lights: SpatialIndex,
    view: Option<(Pos<Meters>, Meters)>,
    stood: Stood,
    settled: bool,
}

#[derive(Default)]
struct Stood {
    ground: HashMap<Node, Entity>,
    covers: HashMap<(usize, usize, (u32, u32)), Entity>,
    props: HashMap<usize, Entity>,
    lights: HashMap<usize, Entity>,
}

/// A cover a terrain grows, ready to stand chunk by chunk: its slot among the area's terrains, its
/// pieces, the material its sheets and blades are drawn with, and the cover's seed.
struct CoverLayer {
    slot: usize,
    seed: u64,
    sheet_level: Option<f32>,
    ground_color: LinearRgba,
    pieces: Vec<LayerPiece>,
    blended: Handle<CoverMaterial>,
}

/// A piece of a cover as its layer stands it: its shape (none for the sheet) and how it is
/// spread, and for a prop the model it strews and the material its copies are drawn with.
struct LayerPiece {
    shape: Option<CoverPiece>,
    spread: cover::Spread,
    strewn: Option<Strewn>,
}

struct Strewn {
    gltf: Handle<Gltf>,
    material: Handle<CoverMaterial>,
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
const SPLIT_WITHIN: f32 = 1.5;
/// How placements are filed to be found near the view.
const PLACEMENT_CELL: Meters = Meters(32.0);
/// How far past its reach the view keeps what stands, so pacing back and forth over the edge
/// stands nothing twice.
const KEEP_SHARE: f32 = 1.25;
/// How much stands in one frame where frames are paced, and among the props how many whose model
/// no earlier prop asked for: a model's load falls on the frames after the prop that starts it.
const CHUNKS_PER_FRAME: usize = 4;
const COVERS_PER_FRAME: usize = 16;
const PROPS_PER_FRAME: usize = 16;
const FRESH_MODELS_PER_FRAME: usize = 1;

fn stream(world: &mut World) {
    let roots: Vec<Entity> = world
        .query_filtered::<Entity, With<Streaming>>()
        .iter(world)
        .collect();
    let framed = world.resource::<Pace>().is_framed();
    for root in roots {
        let Some(mut streaming) = world.entity_mut(root).take::<Streaming>() else {
            continue;
        };
        let Some((at, reach)) = streaming.view else {
            world.entity_mut(root).insert(streaming);
            continue;
        };
        let keep = Meters(reach.0 * KEEP_SHARE);
        let mut budget = Budget::new(framed);
        stream_ground(world, &mut streaming, at, &mut budget);
        stream_covers(world, &mut streaming, root, at, reach, keep, &mut budget);
        stream_props(world, &mut streaming, root, at, reach, keep, &mut budget);
        stream_lights(world, &mut streaming, root, at, reach, keep);
        streaming.settled = !budget.spent;
        world.entity_mut(root).insert(streaming);
    }
}

/// What a frame may still stand, where frames are paced.
struct Budget {
    framed: bool,
    chunks: usize,
    covers: usize,
    props: usize,
    fresh: usize,
    /// Whether the frame left something to stand.
    spent: bool,
}

impl Budget {
    fn new(framed: bool) -> Budget {
        Budget {
            framed,
            chunks: CHUNKS_PER_FRAME,
            covers: COVERS_PER_FRAME,
            props: PROPS_PER_FRAME,
            fresh: FRESH_MODELS_PER_FRAME,
            spent: false,
        }
    }

    fn take(&mut self, left: fn(&mut Budget) -> &mut usize) -> bool {
        if !self.framed {
            return true;
        }
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
    let wanted = nodes_for(&streaming.heights, at);
    let gone: Vec<Node> = streaming
        .stood
        .ground
        .keys()
        .filter(|node| !wanted.contains(node))
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
                world.despawn(entity);
            }
        }
        return;
    }
    missing.sort_by_key(|node| node.level);
    for node in missing {
        if !budget.take(|budget| &mut budget.chunks) {
            break;
        }
        let entity = spawn_chunk(world, &streaming.heights, node, streaming.terrain);
        streaming.stood.ground.insert(node, entity);
    }
    if wanted
        .iter()
        .all(|node| streaming.stood.ground.contains_key(node))
    {
        for node in gone {
            if let Some(entity) = streaming.stood.ground.remove(&node) {
                world.despawn(entity);
            }
        }
    }
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

fn spawn_chunk(world: &mut World, heights: &Heightfield, node: Node, terrain: Entity) -> Entity {
    let mesh = chunk_mesh(heights, node);
    let mesh = world.resource_mut::<Assets<Mesh>>().add(mesh);
    let material = world
        .get::<MeshMaterial3d<GroundMaterial>>(terrain)
        .map(|material| material.0.clone())
        .expect("the ground's material");
    world
        .spawn((
            Mesh3d(mesh),
            MeshMaterial3d(material),
            Transform::default(),
            ChildOf(terrain),
        ))
        .id()
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

fn stream_covers(
    world: &mut World,
    streaming: &mut Streaming,
    root: Entity,
    at: Pos<Meters>,
    reach: Meters,
    keep: Meters,
    budget: &mut Budget,
) {
    let (columns, rows) = streaming.paint.pixels();
    let chunk_side = cover::CHUNK_CELLS as f32 / SAMPLES_PER_METER as f32;
    let chunk_bounds = |chunk: (u32, u32)| {
        Rect::new(
            Paint::pixel_pos(chunk.0 * cover::CHUNK_CELLS, chunk.1 * cover::CHUNK_CELLS),
            Size::new(chunk_side, chunk_side),
        )
    };
    let gone: Vec<_> = streaming
        .stood
        .covers
        .keys()
        .filter(|(.., chunk)| distance_to(chunk_bounds(*chunk), at) > keep.0)
        .copied()
        .collect();
    for key in gone {
        if let Some(entity) = streaming.stood.covers.remove(&key) {
            world.despawn(entity);
        }
    }
    let first = (
        ((at.x - reach.0) / chunk_side).floor().max(0.0) as u32,
        ((at.y - reach.0) / chunk_side).floor().max(0.0) as u32,
    );
    let last = (
        (((at.x + reach.0) / chunk_side).floor().max(0.0) as u32)
            .min(columns.div_ceil(cover::CHUNK_CELLS).saturating_sub(1)),
        (((at.y + reach.0) / chunk_side).floor().max(0.0) as u32)
            .min(rows.div_ceil(cover::CHUNK_CELLS).saturating_sub(1)),
    );
    for layer in 0..streaming.covers.len() {
        for piece in 0..streaming.covers[layer].pieces.len() {
            for chunk_row in first.1..=last.1 {
                for chunk_column in first.0..=last.0 {
                    let chunk = (chunk_column, chunk_row);
                    let key = (layer, piece, chunk);
                    if streaming.stood.covers.contains_key(&key)
                        || distance_to(chunk_bounds(chunk), at) > reach.0
                    {
                        continue;
                    }
                    if !budget.take(|budget| &mut budget.covers) {
                        return;
                    }
                    let entity = spawn_cover_chunk(world, streaming, layer, piece, chunk, root)
                        .unwrap_or_else(|| world.spawn(ChildOf(root)).id());
                    streaming.stood.covers.insert(key, entity);
                }
            }
        }
    }
}

fn stream_props(
    world: &mut World,
    streaming: &mut Streaming,
    root: Entity,
    at: Pos<Meters>,
    reach: Meters,
    keep: Meters,
    budget: &mut Budget,
) {
    let area = streaming.area.clone();
    let gone: Vec<usize> = streaming
        .stood
        .props
        .keys()
        .filter(|&&index| far(area.props[index].at, at, keep))
        .copied()
        .collect();
    for index in gone {
        if let Some(entity) = streaming.stood.props.remove(&index) {
            world.despawn(entity);
        }
    }
    let mut near: Vec<usize> = streaming
        .props
        .near(at, reach)
        .filter(|&index| {
            !streaming.stood.props.contains_key(&index) && !far(area.props[index].at, at, reach)
        })
        .collect();
    near.sort_by(|&a, &b| {
        let apart = |index: usize| (area.props[index].at - at).length();
        apart(a).total_cmp(&apart(b))
    });
    for index in near {
        if !budget.take(|budget| &mut budget.props) {
            return;
        }
        let placed = &area.props[index];
        if budget.framed {
            let model = placed.prop.get(&streaming.content).model.path();
            let scene = GltfAssetLabel::Scene(0).from_asset(model);
            if world.resource::<AssetServer>().get_path_id(scene).is_none()
                && !budget.take(|budget| &mut budget.fresh)
            {
                return;
            }
        }
        let entity = spawn_prop(world, &streaming.content, &streaming.heights, placed, root);
        streaming.stood.props.insert(index, entity);
    }
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
    let sun = sun_at(area, daylight);
    let elevation = sun.map(|sun| sun.elevation);
    let ambient = AmbientLight {
        color: area.lighting.ambient.color(),
        brightness: AMBIENT_NITS
            * sky::ambient_at(daylight, elevation, area.weather.clouds, area.weather.water),
        ..default()
    };
    world
        .resource_mut::<CloudSheet>()
        .cover(area.weather.clouds, area.weather.water);
    let shining = sun.map(|sun| sky::shining(sun, area.weather.clouds, area.weather.water));
    if let (Some(sun), Some(shining), Some(entity)) = (sun, shining, scenery.sun) {
        if let Some(mut light) = world.get_mut::<DirectionalLight>(entity) {
            light.color = shining.color;
            light.illuminance = shining.illuminance;
        }
        let drifted = clouds::drift(climate(area).wind, world.resource::<Time>().elapsed_secs());
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
                shining.illuminance
                    / (SUN_LUX * sky::clouded(area.weather.clouds, area.weather.water)),
                -lights::toward(sun.elevation, sun.azimuth),
                sky::night(daylight, sun),
                area.weather.clouds,
                drifted,
            );
            light_dome(world, dome, heavens);
        }
    }
    let mut tone = area.lighting.grade.tone();
    let mut lit = 1.0;
    if let (Some(shining), Some(noon)) = (shining, sun_at(area, Daylight::Fixed)) {
        let opened = sky::adaptation(daylight, shining);
        tone.exposure.0 += opened.0;
        let sky_light =
            sky::ambient_at(daylight, elevation, area.weather.clouds, area.weather.water)
                / sky::ambient_at(
                    Daylight::Fixed,
                    None,
                    area.weather.clouds,
                    area.weather.water,
                );
        lit = sky::lit(shining, sky_light, opened, noon);
    }
    if let Ok(mut camera) = world.get_entity_mut(camera) {
        camera.insert((
            ambient,
            sky::haze(sun, lit, area.lighting.sky, humidity(area)),
            camera::grading(tone),
        ));
    }
    sky::show(world, camera, area.lighting.sky);
}

/// How moist the area's air is under its weather.
fn humidity(area: &Area) -> f32 {
    sky::humidity(area.weather.clouds, area.weather.water)
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
        weather: world.resource::<cover_vfx::WeatherImage>().0.clone(),
        heavens: Heavens::default(),
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

fn sun_at(area: &Area, daylight: Daylight) -> Option<SunLight> {
    area.lighting.sun.map(|sun| {
        sky::sun_at(
            daylight,
            sun.elevation,
            sun.azimuth,
            sun.color.color(),
            area.weather.clouds,
            area.weather.water,
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
pub fn lay_ground(world: &mut World, scenery: Scenery, heights: &Heightfield, paint: &Paint) {
    let Some(mut streaming) = world.entity_mut(scenery.root).take::<Streaming>() else {
        return;
    };
    streaming.heights = heights.clone();
    streaming.paint = paint.clone();
    for (&node, &entity) in &streaming.stood.ground {
        let mesh = chunk_mesh(&streaming.heights, node);
        let mesh = world.resource_mut::<Assets<Mesh>>().add(mesh);
        if let Ok(mut chunk) = world.get_entity_mut(entity) {
            chunk.insert(Mesh3d(mesh));
        }
    }
    let covers: Vec<Entity> = streaming
        .stood
        .covers
        .drain()
        .map(|(_, entity)| entity)
        .collect();
    for entity in covers {
        world.despawn(entity);
    }
    world.entity_mut(scenery.root).insert(streaming);
    let Some(material) = world
        .get::<MeshMaterial3d<GroundMaterial>>(scenery.terrain)
        .map(|material| material.0.clone())
    else {
        return;
    };
    world.resource_scope(|world, mut images: Mut<Assets<Image>>| {
        let mut materials = world.resource_mut::<Assets<GroundMaterial>>();
        terrain::repaint(&material, paint, &mut materials, &mut images);
    });
}

/// The ground's root, holding the material its chunks are painted with.
fn spawn_terrain(world: &mut World, content: &Content, area: &Area, root: Entity) -> Entity {
    let layers = ground_layers(content, area);
    let (material, pictures) = world.resource_scope(|world, mut images: Mut<Assets<Image>>| {
        terrain::ground_material(
            &layers,
            &area.paint,
            world.resource::<AssetServer>(),
            &mut images,
        )
    });
    let material = world.resource_mut::<Assets<GroundMaterial>>().add(material);
    world
        .spawn((
            MeshMaterial3d(material),
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
            }
        })
        .collect()
}

/// Each terrain with a cover, readied to lay it chunk by chunk as the view comes: the material
/// its sheets and blades are drawn with, through the terrain's cover shader with the area as its
/// site, and its pieces with how each is spread, a prop piece with its model loading.
fn cover_layers(world: &mut World, content: &Content, area: &Area) -> Vec<CoverLayer> {
    let bounds = area.size.bounds();
    let site = PlacedShape {
        at: Pos::new(
            bounds.origin.x + bounds.size.width / 2.0,
            bounds.origin.y + bounds.size.height / 2.0,
        ),
        facing: Radians::default(),
        shape: Shape::Rect {
            width: Meters(bounds.size.width),
            height: Meters(bounds.size.height),
        },
    };
    let models = content.built::<GltfInfos>();
    let prop_bounds = |prop: PropId| {
        let bounds = models.0.get(&prop.get(content).model.path())?.bounds;
        Some((bounds.max.x - bounds.min.x, bounds.max.z - bounds.min.z))
    };
    let mut layers = Vec::new();
    for (slot, terrain) in area.terrains.iter().enumerate() {
        let ground = terrain.get(content);
        let Some(cover_def) = &ground.cover else {
            continue;
        };
        let Some(shader) = vfx_render::compiled_shader(world, cover_def.shader) else {
            continue;
        };
        let frame = SiteFrame::of_area(&site, |at| area.heights.height_at(at));
        let run = Run {
            progress: 0.0,
            time: Seconds(0.0),
        };
        let params = ParamsUniform::pack(cover_def.shader.get(content), |name| {
            match cover_def.params.get(name)? {
                ParamFill::Value(value) => Some(*value),
                ParamFill::Param { .. } => None,
            }
        });
        let material = |world: &mut World, strewn: Option<&cover::Spread>| {
            let weather = world.resource::<cover_vfx::WeatherImage>().0.clone();
            world
                .resource_mut::<Assets<CoverMaterial>>()
                .add(cover_vfx::material(
                    frame.uniform(run, Stage::Cover),
                    params,
                    shader.clone(),
                    weather,
                    strewn,
                ))
        };
        let blended = material(world, None);
        let ground_color = ground.color.color().to_linear();
        let sheet = cover_def.sheet.map(|_| LayerPiece {
            shape: None,
            spread: cover::Spread {
                piece: 0,
                per_square_meter: 0.0,
                size: (1.0, 1.0),
                color: (ground_color, ground_color),
            },
            strewn: None,
        });
        let pieces: Vec<LayerPiece> = sheet
            .into_iter()
            .chain(cover_def.pieces.iter().enumerate().map(|(index, piece)| {
                let spread = cover::Spread {
                    piece: index as u32,
                    per_square_meter: piece.per_square_meter(piece.shape.base_area(prop_bounds)),
                    size: (piece.size.min, piece.size.max),
                    color: (
                        piece.color.from.color().to_linear(),
                        piece.color.to.color().to_linear(),
                    ),
                };
                let strewn = match &piece.shape {
                    PieceShape::Prop { prop } => Some(Strewn {
                        gltf: world
                            .resource::<AssetServer>()
                            .load(prop.get(content).model.path()),
                        material: material(world, Some(&spread)),
                    }),
                    PieceShape::Blade { .. } => None,
                };
                LayerPiece {
                    shape: Some(piece.clone()),
                    spread,
                    strewn,
                }
            }))
            .collect();
        layers.push(CoverLayer {
            slot,
            seed: cover_def.seed,
            sheet_level: cover_def
                .sheet
                .and_then(|sheet| sheet.level)
                .map(|level| level.0),
            ground_color,
            pieces,
            blended,
        });
    }
    layers
}

/// A chunk of a cover's piece, stood under `root`: its sheet or blades, or for a prop the copies
/// it strews once the prop has loaded; none where the chunk holds nothing of the piece.
fn spawn_cover_chunk(
    world: &mut World,
    streaming: &Streaming,
    layer: usize,
    piece: usize,
    chunk: (u32, u32),
    root: Entity,
) -> Option<Entity> {
    let layer = &streaming.covers[layer];
    let (columns, rows) = streaming.paint.pixels();
    let share = |column: u32, row: u32| {
        if usize::from(streaming.paint.index(column, row)) == layer.slot {
            1.0
        } else {
            0.0
        }
    };
    let first = (chunk.0 * cover::CHUNK_CELLS, chunk.1 * cover::CHUNK_CELLS);
    let patch = cover::Patch {
        heights: &streaming.heights,
        first,
        columns: cover::CHUNK_CELLS.min(columns - first.0),
        rows: cover::CHUNK_CELLS.min(rows - first.1),
        share: &share,
    };
    let LayerPiece {
        shape,
        spread,
        strewn,
    } = &layer.pieces[piece];
    let seed = chunk_seed(
        layer.seed,
        [
            layer.slot as u64,
            u64::from(spread.piece),
            u64::from(chunk.1),
            u64::from(chunk.0),
        ],
    );
    let mesh = match shape.as_ref().map(|piece| &piece.shape) {
        None => cover::sheet(&patch, layer.sheet_level, layer.ground_color),
        Some(PieceShape::Blade { width, height }) => cover::blades(
            &patch,
            &cover::BladeShape {
                width: width.0,
                height: height.0,
            },
            spread,
            seed,
        ),
        Some(PieceShape::Prop { .. }) => {
            let Strewn { gltf, material } = strewn.as_ref()?;
            let places = cover::strew_places(&patch, spread, seed);
            if places.is_empty() {
                return None;
            }
            return Some(
                world
                    .spawn((
                        cover::Strewing {
                            gltf: gltf.clone(),
                            places,
                            material: material.clone(),
                        },
                        Transform::default(),
                        Visibility::default(),
                        ChildOf(root),
                    ))
                    .id(),
            );
        }
    }?;
    let bounds = mesh.get_aabb()?;
    let mesh = world.resource_mut::<Assets<Mesh>>().add(mesh);
    Some(
        world
            .spawn((
                Mesh3d(mesh),
                MeshMaterial3d(layer.blended.clone()),
                bounds,
                NotShadowCaster,
                ChildOf(root),
            ))
            .id(),
    )
}

/// A seed of its own for every chunk of every piece, mixed from the cover's seed and where the
/// chunk lies, so changing the cover's seed rescatters everything and nothing else repeats.
fn chunk_seed(seed: u64, parts: [u64; 4]) -> u64 {
    let mut mixed = seed ^ 0x9E37_79B9_7F4A_7C15;
    for part in parts {
        mixed ^= part.wrapping_add(0x9E37_79B9_7F4A_7C15);
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        mixed ^= mixed >> 31;
    }
    mixed
}

/// The transform a prop placed at `placed` stands with on `heights`.
pub fn prop_transform(heights: &Heightfield, placed: &PropPlacement) -> Transform {
    Transform {
        translation: world_point(placed.at, heights.height_at(placed.at) - placed.sunk),
        rotation: Quat::from_rotation_y(-placed.yaw.radians().0),
        scale: Vec3::splat(placed.scale),
    }
}

fn spawn_prop(
    world: &mut World,
    content: &Content,
    heights: &Heightfield,
    placed: &PropPlacement,
    root: Entity,
) -> Entity {
    let def = placed.prop.get(content);
    let scene = world
        .resource::<AssetServer>()
        .load(GltfAssetLabel::Scene(0).from_asset(def.model.path()));
    let looks = MaterialLooks(
        def.materials
            .iter()
            .filter_map(|(name, material)| {
                let material = material.get(content);
                Some(MaterialLook {
                    material: name.clone(),
                    shader: vfx_render::compiled_shader(world, material.shader)?,
                    params: ParamsUniform::pack(material.shader.get(content), |param| {
                        material.params.get(param).copied()
                    }),
                    translucency: material.translucency,
                })
            })
            .collect(),
    );
    let mut prop = world.spawn((
        WorldAssetRoot(scene),
        prop_transform(heights, placed),
        looks,
        ChildOf(root),
    ));
    if def.fade {
        prop.insert(Fades::default());
    }
    if !def.shadow {
        prop.insert(NotShadowCaster);
    }
    prop.id()
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
