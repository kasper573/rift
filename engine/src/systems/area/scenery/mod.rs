//! What stands in a world for an area: its ground as coloured meshes of its chunks, the covers
//! its terrains grow, its props on the ground, its point lights, its sun, the sky its sun lights
//! and the air over it, every one a child of one root. The client stands the area the player is
//! in; the editor stands the area it edits, on render layers of its own, and both light it alike
//! for the hour. The ground stands whole, finer the nearer the view, and the covers, props and
//! lights stand within the view's reach alone, a few a frame through `SceneryPlugin` as the view
//! moves, the props and the covers' copies in the GPU's arenas chunk by chunk, thinned far off the
//! eye as the view shows them, so an area costs what the view takes in, however far it stretches.

mod covers;
pub mod expand;
mod grass;
mod instances;
mod plan;
mod props;
mod waters;

pub use instances::impostor_of;
pub use plan::{Eyes, Sight};
pub use props::{ShowsProp, compiled_looks, extent_of, prop_transform};

use covers::Covers;
use grass::Grass;
use instances::Instances;
use plan::{Plan, Planner, Seen};

use std::collections::{BTreeMap, BTreeSet};
use std::f32::consts::PI;
use std::ops::Bound;
use std::sync::Arc;

use bevy::app::Propagate;
use bevy::camera::visibility::RenderLayers;
use bevy::ecs::component::Mutable;
use bevy::light::atmosphere::ScatteringMedium;
use bevy::light::{FogVolume, SunDisk};
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, IoTaskPool, Task};

use super::heavens::{self, Heavens, RainMaterial, SkyMaterial, SkyWeather};
use super::lighting::DomeLook;
use super::prop::{PropDef, PropId, PropPlacement};
use super::rain::RainField;
use super::shown_weather::{ShownWeather, ShownWeatherPlugin};
use super::terrain::PieceShape;
use super::{Area, AreaModule, Climate, LaidFog, Lighting, PointLightDef, Rainfall, Weather};
use crate::core::content::{Content, ContentFile, FileHash, FileHashes, ImageRef};
use crate::core::ground::{
    GroundGrid, GroundValue, Heightfield, OVERVIEW_STRIDE, PaintMap, SAMPLES_PER_METER, TILE_SIDE,
    TileAt, TileVersion, WaterMap, WaterSurface,
};
use crate::core::jobs::{self, Handoff};
use crate::core::math::{Pos, Rect, Rng, Size};
use crate::core::render::camera::WorldPicture;
use crate::core::render::present::AreaGrade;
use crate::core::render::sample::FrameWork;
use crate::core::render::video::VideoPreferences;
use crate::core::render::world::SIGHT_LENGTHS;
use crate::core::render::world::backdrop::{BackdropMaterial, backdrop};
use crate::core::render::world::clouds::{CloudDrift, CloudSheet};
use crate::core::render::world::gpu::arena::PropArena;
use crate::core::render::world::gpu::atlas::TerrainAtlas;
use crate::core::render::world::gpu::draw::{GroundDraw, GroundPatch, PATCH_CELLS, SheetOver};
use crate::core::render::world::haze::{
    Haze, HazeBox, HazeLight, HazeSight, HazeSky, MAX_HAZE_BOXES,
};
use crate::core::render::world::lights::{self, Lamp};
use crate::core::render::world::opacity::Opacity;
use crate::core::render::world::sky::{self, Daylight, SkyAir, SkyLights, SkyLook, SunLight};
use crate::core::render::world::sky_picture::{self, SkyPicture};
use crate::core::render::world::terrain::{self, GroundLayer};
use crate::core::render::world::water;
use crate::core::render::world::{DRAWN_FRAMES_AFTER, frames_drawn, missing_drawn};
use crate::core::render::{CameraRig, Look, camera, world_point};
use crate::core::spatial::SpatialIndex;
use crate::core::units::{MeterSize, Meters};
use crate::core::vfx::board::ShaderBoardImage;
use crate::core::vfx::render as vfx_render;
use crate::core::vfx::uniform::{ParamsUniform, ShaderValue, SiteUniform};
use crate::systems::area::sky::SkyCondition;
use crate::systems::area::sky::SkyNow;
use crate::systems::water::WaterId;

/// An area's scenery as it stands in a world: everything hangs under `root`, the terrain mesh is
/// `terrain`, `sun` is the light that casts its shadows when the area has a sun, the sun by day
/// and the full moon by night, `second` the other of the two, which lights the air and casts
/// none, `atmosphere` the air over it, `sky` the dome its sky is painted on when the area paints
/// one, `picture` the picture drawn of that sky, and `rain` the shells its rain falls on when its
/// climate draws rain under an open sky.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scenery {
    pub root: Entity,
    pub terrain: Entity,
    pub sun: Option<Entity>,
    pub second: Option<Entity>,
    pub atmosphere: Entity,
    pub sky: Option<Entity>,
    pub picture: Option<PicturedSky>,
    pub air: Option<Entity>,
    pub rain: Option<Entity>,
}

/// A sky's picture being drawn: the camera that draws it and the face it paints the sky on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PicturedSky {
    pub camera: Entity,
    pub face: Entity,
}

/// Stands scenery as the view moves, a few pieces a frame, so the frames keep coming however much
/// the view takes in at once.
pub struct SceneryPlugin;

impl Plugin for SceneryPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ShownWeatherPlugin)
            .init_resource::<TakenDown>()
            .init_resource::<Streams>()
            .add_systems(
                Update,
                (
                    regrow_grass.run_if(resource_changed::<VideoPreferences>),
                    take_down,
                    stream,
                    light_by_sky.run_if(resource_exists::<SkyNow>),
                )
                    .chain(),
            )
            .add_systems(Update, props::show_props.run_if(resource_exists::<Content>));
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
            Propagate(layers.clone()),
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
    let second = area.lighting.sun.map(|sun| {
        let unlit = SunLight {
            elevation: sun.elevation,
            azimuth: sun.azimuth,
            color: sun.color.color(),
            illuminance: 0.0,
        };
        world.spawn((lights::unshadowed(unlit), ChildOf(root))).id()
    });
    let look = *world.resource::<Look>();
    let atmosphere = sky::atmosphere(
        &mut world.resource_mut::<Assets<ScatteringMedium>>(),
        area.lighting.sky,
        area.lighting.scattering(0.0),
        &look.sky,
    );
    let atmosphere = world.spawn((atmosphere, ChildOf(root))).id();
    let mut dome = None;
    let mut picture = None;
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
        picture = dome.and_then(|dome| picture_sky(world, dome, &layers, root));
        let far = world.resource_scope(|world, mut materials: Mut<Assets<BackdropMaterial>>| {
            backdrop(&mut world.resource_mut::<Assets<Mesh>>(), &mut materials)
        });
        world.spawn((far, ChildOf(root)));
        rain = area
            .climate
            .rain
            .as_ref()
            .and_then(|rainfall| spawn_rain(world, content, rainfall, root));
    }
    let terrain = spawn_terrain(world, content, area, &look, root);
    let covers = Covers::new(world, content, area);
    let sky = picture
        .as_ref()
        .and_then(|picture| world.get::<SkyPicture>(picture.camera))
        .map(|picture| picture.image.clone());
    let mut sheets = covers.sheets();
    sheets.extend(waters::sheets(
        world,
        content,
        area,
        &area.waters,
        sky.as_ref(),
    ));
    if let Some(mut draw) = world.get_mut::<GroundDraw>(terrain) {
        draw.sheets = sheets.into();
    }
    let grass = Grass::new(world, terrain, covers.grass());
    let planner = planner_of(area);
    world.entity_mut(root).insert(PropArena {
        floor: area.heights.range().lowest,
    });
    let streaming = Streaming {
        content: content.clone(),
        area: area.clone(),
        heights: area.heights.clone(),
        paint: area.paint.clone(),
        water: area.water.clone(),
        water_map: area.water_map.clone(),
        waters: area.waters.clone(),
        mirrored: sky,
        terrain,
        grass,
        instances: Instances::new(content, area, covers.copies()),
        planner,
        sight: None,
        seen: false,
        planning: None,
        plan: None,
        whole: false,
        stood_at: None,
        stood: Stood::default(),
        drawn: None,
        shortfalls: Shortfalls::default(),
        loading: HashMap::default(),
    };
    world.resource_mut::<Streams>().0.insert(root, streaming);
    Scenery {
        root,
        terrain,
        sun,
        second,
        atmosphere,
        sky: dome,
        picture,
        air,
        rain,
    }
}

/// Tells the scenery the sight it stands for: the covers, props and lights within its reach, a
/// cover's pieces only where they span a pixel from the nearest its eyes may come; which props
/// each view shows, and how finely, the GPU works out frame by frame. The ground stands whole,
/// finer the nearer the ground the sight looks at. What stands follows over the frames.
pub fn view_from(world: &mut World, scenery: Scenery, sight: Sight) {
    if let Some(streaming) = world
        .resource_mut::<Streams>()
        .bypass_change_detection()
        .0
        .get_mut(&scenery.root)
    {
        streaming.sight = Some(sight);
        streaming.seen = true;
    }
}

/// Tells the scenery no eye looks at it until it is viewed again.
pub fn unseen(world: &mut World, scenery: Scenery) {
    if let Some(streaming) = world
        .resource_mut::<Streams>()
        .bypass_change_detection()
        .0
        .get_mut(&scenery.root)
    {
        streaming.seen = false;
    }
}

/// Stands `area`, read from `content`, in place of the area `scenery` stands, keeping all both
/// stand alike: the props of every chunk where none moved, came or went, the ground and what
/// grows on it wherever its tiles hold what they held, its waters, and the lights while none
/// changed. False, changing nothing, when the two differ in what only standing anew takes in: the
/// ground's size, the terrains, lighting or climate, or a file beyond the area both are made from.
pub fn restand(world: &mut World, scenery: Scenery, content: &Content, area: &Arc<Area>) -> bool {
    world.resource_scope(|world, mut streams: Mut<Streams>| {
        let Some(streaming) = streams.0.get_mut(&scenery.root) else {
            return false;
        };
        if Arc::ptr_eq(&streaming.area, area) {
            return true;
        }
        let before = streaming.area.clone();
        let was = &streaming.content;
        let hashes = (
            &was.built::<FileHashes>().0,
            &content.built::<FileHashes>().0,
        );
        let terrains = |content: &Content, area: &Area| -> Vec<String> {
            area.terrains
                .iter()
                .map(|terrain| terrain.path(content).to_owned())
                .collect()
        };
        let waters = |content: &Content, area: &Area| -> Vec<String> {
            area.waters
                .iter()
                .map(|water| water.path(content).to_owned())
                .collect()
        };
        let alike = area.size == before.size
            && area.lighting == before.lighting
            && area.climate == before.climate
            && area.rain == before.rain
            && same_paths::<PropDef>(was, content)
            && terrains(was, &before) == terrains(content, area)
            && Sources::of(was, &before).agree(&Sources::of(content, area));
        if !alike {
            return false;
        }
        let heights = area.heights.keeping(&streaming.heights, |at| {
            tile_alike(&streaming.heights, &area.heights, at, hashes)
        });
        let paint = area.paint.keeping(&streaming.paint, |at| {
            tile_alike(&streaming.paint, &area.paint, at, hashes)
        });
        let water = area.water.keeping(&streaming.water, |at| {
            tile_alike(&streaming.water, &area.water, at, hashes)
        });
        let water_map = area.water_map.keeping(&streaming.water_map, |at| {
            tile_alike(&streaming.water_map, &area.water_map, at, hashes)
        });
        if waters(was, &before) != waters(content, area) {
            lay_waters(world, streaming, content, &area.waters);
        }
        streaming.waters.clone_from(&area.waters);
        lay_streamed_ground(
            world,
            streaming,
            ShapedGround {
                heights: &heights,
                paint: &paint,
                water: &water,
                water_map: &water_map,
                waters: &area.waters,
            },
        );
        let (old, new) = (&before.prop_sources, &area.prop_sources);
        let first = old
            .iter()
            .zip(new)
            .take_while(|(a, b)| same_source(a, b))
            .count();
        let last = old[first..]
            .iter()
            .rev()
            .zip(new[first..].iter().rev())
            .take_while(|(a, b)| same_source(a, b))
            .count();
        let (old_end, new_end) = (old.len() - last, new.len() - last);
        let touched: Vec<Rect<Meters>> = old[first..old_end]
            .iter()
            .chain(&new[first..new_end])
            .filter_map(stretch_of)
            .chain(
                area.scatter_clearing
                    .changes_from(&before.scatter_clearing)
                    .map(|taken| taken.inflate(EDGE_REACH.0, EDGE_REACH.0)),
            )
            .collect();
        streaming
            .instances
            .sources_changed(area, area.size.bounds(), &touched, |source| {
                let source = source as usize;
                if source < first {
                    Some(source as u32)
                } else if source >= old_end {
                    Some((source - old_end + new_end) as u32)
                } else {
                    None
                }
            });
        let acting = |area: &Area| -> Vec<PropPlacement> {
            area.acting
                .iter()
                .map(|&index| area.props[index].clone())
                .collect()
        };
        if area.lights != before.lights || acting(area) != acting(&before) {
            for (_, stood) in streaming
                .stood
                .lights
                .drain()
                .chain(streaming.stood.acting.drain())
            {
                world.despawn(stood);
            }
            streaming.planner = planner_of(area);
            streaming.planning = None;
            streaming.plan = None;
        }
        let replaced = (
            std::mem::replace(&mut streaming.area, area.clone()),
            std::mem::replace(&mut streaming.content, content.clone()),
        );
        jobs::drop_off_thread(replaced);
        streaming.whole = false;
        true
    })
}

/// The files the scenery of an area is made from beyond the area itself, each with what it held
/// when read: its terrains, their covers' shaders, its waters and their shaders, the props it
/// stands and strews, their models, and the pictures and shaders their materials are drawn with. Scenery stood from equal sources looks alike, so it need
/// not stand again.
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
        for water in &area.waters {
            files.insert(water.path(content));
            files.insert(water.get(content).shader.path(content));
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
                let material = material.get(content);
                files.extend(material.shader.map(|shader| shader.path(content)));
                files.extend(material.color.as_ref().map(ImageRef::as_str));
                files.extend(material.normal.as_ref().map(ImageRef::as_str));
                files.extend(
                    material
                        .wear
                        .iter()
                        .flat_map(|wear| [wear.mask.as_str(), wear.mottle.as_str()]),
                );
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

    /// Whether every file both are made from holds the same in each: what one reads beyond the
    /// other may differ.
    pub fn agree(&self, other: &Sources) -> bool {
        let theirs: BTreeMap<&str, &FileHash> = other
            .0
            .iter()
            .map(|(path, hash)| (path.as_str(), hash))
            .collect();
        self.0.iter().all(|(path, hash)| {
            theirs
                .get(path.as_str())
                .is_none_or(|theirs| *theirs == hash)
        })
    }
}

/// The path just past every part of `file`'s bundle in path order: after `file` itself and
/// `file/…` comes the next character above the separator.
fn file_bound(file: &str) -> String {
    format!("{file}0")
}

/// The props standing in the area, each once.
pub(super) fn props_of(area: &Area) -> Vec<PropId> {
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
                material
                    .get(content)
                    .shader
                    .is_none_or(|shader| vfx_render::compiled_shader(world, shader).is_some())
            })
        })
}

/// Whether any scenery an eye looks at has yet to stand what its sight takes in.
pub fn standing(world: &World) -> bool {
    sceneries_standing(world) > 0
}

/// How many sceneries an eye looks at have yet to stand what their sight takes in.
pub fn sceneries_standing(world: &World) -> u32 {
    let drawn = frames_drawn(world);
    world
        .resource::<Streams>()
        .0
        .values()
        .filter(|streaming| streaming.seen && !streaming.stands(drawn))
        .count() as u32
}

/// How much of the ground, the props, the covers' copies and their blades the sights take in has
/// yet to stand at all.
pub fn missing(world: &World) -> u32 {
    shortfalls(world).total().missing + missing_drawn(world)
}

/// How much of each part of what the sights take in has yet to stand as it is now, over every
/// scenery streaming.
pub fn shortfalls(world: &World) -> Shortfalls {
    let mut all = Shortfalls::default();
    for streaming in world.resource::<Streams>().0.values() {
        if streaming.seen {
            all += streaming.shortfalls;
        }
    }
    all
}

/// How much of what a plan takes in has yet to stand as it is now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Shortfall {
    /// Not standing at all, or coarser than wanted, where the sights take it in.
    pub missing: u32,
    /// Standing as it did before an edit, its new values on their way.
    pub stale: u32,
    /// Not standing yet, or coarser or older than wanted, beyond what the sights take in: the
    /// plan stands past their edges so that what they move onto stands before they reach it.
    pub ahead: u32,
}

impl Shortfall {
    pub fn any(self) -> bool {
        self.missing > 0 || self.stale > 0 || self.ahead > 0
    }
}

impl std::ops::AddAssign for Shortfall {
    fn add_assign(&mut self, other: Shortfall) {
        self.missing += other.missing;
        self.stale += other.stale;
        self.ahead += other.ahead;
    }
}

/// How much of each part of what a plan takes in has yet to stand as it is now: the ground's
/// tiles, the lots of props, of the covers' copies and of the far field, the canopy over the
/// ground, and the covers' blades.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Shortfalls {
    pub ground: Shortfall,
    pub props: Shortfall,
    pub copies: Shortfall,
    pub far: Shortfall,
    pub canopy: Shortfall,
    pub grass: Shortfall,
}

impl Shortfalls {
    pub fn total(self) -> Shortfall {
        let mut total = Shortfall::default();
        for part in [
            self.ground,
            self.props,
            self.copies,
            self.far,
            self.canopy,
            self.grass,
        ] {
            total += part;
        }
        total
    }
}

impl std::ops::AddAssign for Shortfalls {
    fn add_assign(&mut self, other: Shortfalls) {
        self.ground += other.ground;
        self.props += other.props;
        self.copies += other.copies;
        self.far += other.far;
        self.canopy += other.canopy;
        self.grass += other.grass;
    }
}

/// How many props `scenery` holds in its arena on the GPU.
pub fn arena_props(world: &World, scenery: Scenery) -> u32 {
    world
        .resource::<Streams>()
        .0
        .get(&scenery.root)
        .map_or(0, |streaming| streaming.instances.props())
}

/// Where the module standing the instance at `instance` of `scenery`'s arena was written: the
/// path of the node in the area's scene that is the module or brings it. None for a cover's copy.
pub fn origin_of(world: &World, scenery: Scenery, instance: u32) -> Option<String> {
    let streaming = world.resource::<Streams>().0.get(&scenery.root)?;
    let source = streaming.instances.source_of(instance)?;
    streaming.area.prop_origins.get(source as usize).cloned()
}

/// A number that changes each time the props standing in `scenery`'s arena do, and with it what
/// [`origin_of`] and [`instances_of`] tell.
pub fn props_revision(world: &World, scenery: Scenery) -> Option<u64> {
    let streaming = world.resource::<Streams>().0.get(&scenery.root)?;
    Some(streaming.instances.props_revision())
}

/// Every instance of `scenery`'s arena that a module written at an origin `outlined` takes stands.
pub fn instances_of(world: &World, scenery: Scenery, outlined: impl Fn(&str) -> bool) -> Vec<u32> {
    let Some(streaming) = world.resource::<Streams>().0.get(&scenery.root) else {
        return Vec::new();
    };
    let origins = &streaming.area.prop_origins;
    streaming.instances.standing_for(|source| {
        origins
            .get(source as usize)
            .is_some_and(|origin| outlined(origin))
    })
}

/// How many copies of its covers' pieces `scenery` holds in its arena on the GPU.
pub fn arena_copies(world: &World, scenery: Scenery) -> u32 {
    world
        .resource::<Streams>()
        .0
        .get(&scenery.root)
        .map_or(0, |streaming| streaming.instances.copies())
}

/// How many blades of its covers' grass `scenery` holds on the GPU.
pub fn grass_blades(world: &World, scenery: Scenery) -> u64 {
    world
        .resource::<Streams>()
        .0
        .get(&scenery.root)
        .map_or(0, |streaming| streaming.grass.blades())
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

/// Whether `scenery` stands all its sight takes in, drawn.
pub fn stood(world: &World, scenery: Scenery) -> bool {
    let drawn = frames_drawn(world);
    world
        .resource::<Streams>()
        .0
        .get(&scenery.root)
        .is_some_and(|streaming| streaming.stands(drawn))
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
    water_map: WaterMap,
    /// The waters whose sheets lie and whose looks the ground reads, by their place in the
    /// water map.
    waters: Vec<WaterId>,
    /// The picture of the sky the waters' sheets mirror, where the area has one.
    mirrored: Option<Handle<Image>>,
    terrain: Entity,
    grass: Grass,
    instances: Instances,
    planner: Making<Arc<Planner>>,
    sight: Option<Sight>,
    /// Whether an eye looks at it now: a scenery kept while no view shows it goes on standing
    /// what it can, but its shortfall is no one's wait.
    seen: bool,
    /// The plan being made, for the widened sight it is made for.
    planning: Option<(Sight, Task<Plan>)>,
    plan: Option<Arc<Plan>>,
    /// Whether all the plan takes in stands, so a sight that stays within it costs nothing to
    /// stream.
    whole: bool,
    /// How many frames the render world had drawn when it last came to stand whole.
    stood_at: Option<u64>,
    stood: Stood,
    /// The plan whose ground the terrain draws.
    drawn: Option<Arc<Plan>>,
    /// How much of each part of what the plan takes in has yet to stand as it is now.
    shortfalls: Shortfalls,
    /// The tiles of ground being loaded for what stands on them.
    loading: HashMap<TileAt, Task<()>>,
}

impl Streaming {
    /// Whether all the latest sight takes in stands, and the render world, `drawn` frames in, has
    /// drawn it since, so whatever it draws with has been asked of the GPU.
    fn stands(&self, drawn: Option<u64>) -> bool {
        self.stood()
            && drawn
                .zip(self.stood_at)
                .is_none_or(|(drawn, stood)| drawn >= stood + DRAWN_FRAMES_AFTER)
    }

    /// Whether all the latest sight takes in stands.
    fn stood(&self) -> bool {
        self.whole
            && self.sight.is_some()
            && self.plan.as_ref().map(|plan| plan.seen.sight) == self.sight.map(Sight::widened)
    }

    /// Whether streaming has nothing to do until the sight moves.
    fn idle(&self) -> bool {
        self.sight.is_none() || self.stood()
    }
}

#[derive(Default)]
struct Stood {
    lights: HashMap<usize, Entity>,
    /// What stands for each prop that acts of its own accord, by its place among those.
    acting: HashMap<usize, Entity>,
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
/// How near the view a node of the ground must lie, in its own sides, to be cut into finer ones.
const SPLIT_WITHIN: f32 = 1.5;
/// The level from which a node's stride is the overview's, so it draws from the overview alone;
/// a finer node wants the tiles of ground under it in detail.
const OVERVIEW_LEVEL: u32 = OVERVIEW_STRIDE.trailing_zeros();
/// How placements are filed to be found near the view, and the side of the leaves of the
/// quadtree the props stand over.
const PLACEMENT_CELL: Meters = Meters(32.0);
const PARTS_TAKEN_DOWN_PER_FRAME: usize = 2000;

fn stream(world: &mut World) {
    if world.resource::<Streams>().0.values().all(Streaming::idle) {
        return;
    }
    world.resource_scope(|world, mut streams: Mut<Streams>| {
        for (&root, streaming) in streams.bypass_change_detection().0.iter_mut() {
            replan(world, streaming);
            stream_plan(world, root, streaming);
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
fn stream_plan(world: &mut World, root: Entity, streaming: &mut Streaming) {
    let (Some(_), Some(plan)) = (streaming.planner.made(), streaming.plan.clone()) else {
        return;
    };
    if streaming.whole {
        return;
    }
    streaming
        .loading
        .retain(|_, task| check_ready(task).is_none());
    let ground = draw_ground(world, streaming, &plan);
    let mut shortfalls = instances::stand(world, streaming, &plan, root);
    instances::clear(world, streaming, &plan, root);
    shortfalls.ground = ground;
    shortfalls.canopy = instances::lay_canopy(world, streaming);
    shortfalls.grass = grass::grow(world, streaming, &plan.seen);
    streaming.shortfalls = shortfalls;
    stream_lights(world, streaming, &plan, root);
    stream_acting(world, streaming, &plan, root);
    streaming.whole = !shortfalls.total().any();
    if streaming.whole {
        streaming.stood_at = frames_drawn(world);
    }
}

/// Draws the ground `plan` wants, its nodes finer than the overview in detail as the terrain's
/// atlas takes their tiles in, those any eye may come nearest first; how much of it has yet to
/// reach the GPU as it is now.
fn draw_ground(world: &mut World, streaming: &mut Streaming, plan: &Arc<Plan>) -> Shortfall {
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
    let Some(atlas) = world.get::<TerrainAtlas>(terrain) else {
        return Shortfall::default();
    };
    let mut shortfall = Shortfall {
        missing: u32::from(atlas.overview_missing()),
        stale: atlas.stale(),
        ahead: 0,
    };
    let mut missing = atlas.missing_tiles().peekable();
    if missing.peek().is_none() {
        return shortfall;
    }
    let taken_in: Option<HashSet<TileAt>> = streaming.sight.map(|sight| {
        let size = streaming.heights.size();
        nodes_for(
            &streaming.heights,
            &Seen::of(sight, &streaming.heights),
            0.0,
        )
        .into_iter()
        .filter(|node| node.level < OVERVIEW_LEVEL)
        .flat_map(|node| size.tiles_within(node_rect(node).bounds()))
        .collect()
    });
    for tile in missing {
        if taken_in.as_ref().is_none_or(|taken| taken.contains(&tile)) {
            shortfall.missing += 1;
        } else {
            shortfall.ahead += 1;
        }
    }
    shortfall
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

/// How many tiles of ground a view may want in detail at once, so every tile its plan wants has
/// a slot as soon as it is wanted: the tiles under the nodes finer than the overview, which lie
/// as far as the plan splits an overview node from wherever its eyes may stand, each at most
/// half as wide as that node.
fn detail_slots() -> u32 {
    let overview_node = (LEAF_SAMPLES << OVERVIEW_LEVEL) as f32 / SAMPLES_PER_METER as f32;
    let reach = overview_node * SPLIT_WITHIN + plan::OFF_MIDDLE + plan::AHEAD + overview_node / 2.0;
    let across = (2.0 * reach / TILE_SIDE.0).ceil() as u32 + 1;
    across * across
}

/// The nodes of the quadtree over the ground's samples that stand for `seen` looking at ground
/// within `off` of where it looks: every node is cut in four while it may lie within
/// `SPLIT_WITHIN` of its sides from that ground, with its eyes standing as high above the ground
/// as they may, and is wider than a leaf; what remains covers the ground once over, finer the
/// nearer the view.
fn nodes_for(heights: &Heightfield, seen: &Seen, off: f32) -> Vec<Node> {
    let at = seen.at();
    let above = seen.above(heights.range().highest);
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
        let along = (distance_to(bounds, at) - off).max(0.0);
        let near = along.hypot(above) < bounds.size.width * SPLIT_WITHIN;
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

/// Stands something for each prop `plan` stands that acts of its own accord, for its vfx to
/// play on and its ripples to spread from, and takes down those it no longer keeps.
fn stream_acting(world: &mut World, streaming: &mut Streaming, plan: &Plan, root: Entity) {
    let acting = &plan.acting;
    let gone: Vec<usize> = streaming
        .stood
        .acting
        .keys()
        .filter(|index| !acting.kept.contains(*index))
        .copied()
        .collect();
    for index in gone {
        if let Some(entity) = streaming.stood.acting.remove(&index) {
            world.despawn(entity);
        }
    }
    let near: Vec<usize> = acting
        .near
        .iter()
        .filter(|index| !streaming.stood.acting.contains_key(*index))
        .copied()
        .collect();
    for index in near {
        let placed = &streaming.area.props[streaming.area.acting[index]];
        let entity = props::stand_acting(
            world,
            &streaming.content,
            &streaming.heights,
            &streaming.area.snap_surfaces,
            placed,
            root,
        );
        streaming.stood.acting.insert(index, entity);
    }
}

/// Takes `scenery` out of the view at once, and out of the world a few parts a frame, so a
/// whole area going doesn't hold up a frame. What its streaming held, which may be the last of
/// a whole area, is let go of off the frame.
pub fn despawn(world: &mut World, scenery: Scenery) {
    if let Some(streaming) = world.resource_mut::<Streams>().0.remove(&scenery.root) {
        instances::drop_arena(world, scenery.root);
        grass::drop_grass(world, streaming.terrain);
        jobs::drop_off_thread(streaming);
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

/// Grows every scenery's grass again as the player's settings ask, once they change.
fn regrow_grass(mut streams: ResMut<Streams>) {
    for streaming in streams.0.values_mut() {
        streaming.whole = false;
    }
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

/// Lights `area` for the hour under its weather: its sun and its moon turned, whichever lights
/// the ground the more strongly casting the shadows, the air as moist as the weather, the eye
/// opened to how strongly they shine, its clouds carried on by the wind, and the ambient light,
/// the haze and the area's sky put on `camera`; cheap enough to do every frame. Only what comes
/// out different is written, so what follows the light sees a change only when there is one.
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
    let moist = humidity(weather, look);
    let scattering = area.lighting.scattering(sky::stepped_humidity(moist));
    sky::moisten(
        world,
        scenery.atmosphere,
        area.lighting.sky,
        scattering,
        &look.sky,
    );
    let air = SkyAir::new(area.lighting.sky, scattering, &look.sky);
    let eye = world
        .get::<GlobalTransform>(camera)
        .map(GlobalTransform::translation);
    let ground = eye.map_or(Meters(0.0), |eye| {
        area.heights.height_at(Pos::new(eye.x, eye.z))
    });
    let shone = lights_at(area, weather, daylight, look);
    let landed = shone.map(|lights| lights.landed(&air, ground, &look.sky));
    let elevation = shone.map(|lights| lights.sun.elevation);
    let ambient = AmbientLight {
        color: area.lighting.ambient.color(),
        brightness: look.light.ambient_nits
            * sky::ambient_at(daylight, elevation, clouds, water, look),
        ..default()
    };
    world.resource_mut::<CloudSheet>().cover(clouds, water);
    let shining = landed.map(|landed| landed.stronger());
    if let (Some(shone), Some(landed), Some(shining), Some(entity)) =
        (shone, landed, shining, scenery.sun)
    {
        let sun = shone.sun;
        let sun_casts = landed.sun_stronger();
        let (caster, other) = shone.caster(sun_casts);
        shine(world, entity, caster, sky_disc(sun_casts, &look.sky));
        if let Some(second) = scenery.second {
            let unlit = SunLight {
                illuminance: 0.0,
                ..caster
            };
            let other = other.unwrap_or(unlit);
            shine(world, second, other, sky_disc(!sun_casts, &look.sky));
            hold(
                world,
                second,
                lights::shining_from(other.elevation, other.azimuth),
            );
        }
        let now = world.resource::<Time>().elapsed_secs();
        let drifted =
            world
                .resource_mut::<CloudDrift>()
                .blown(weather.wind.blowing(), now, &look.clouds);
        // The clouds' shadows reach only so far and repeat past it, so they are laid round the
        // eye, wherever in the area it is.
        let middle = area.size.bounds().center();
        let over = eye.map_or(Vec2::new(middle.x, middle.y), |eye| eye.xz());
        let placed = lights::sun_transform(shining.elevation, shining.azimuth, over);
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
        reach_shadows(world, area, entity, camera);
        if scenery.sky.is_some() || scenery.rain.is_some() {
            let full = look.light.sun_lux * sky::clouded(clouds, water, look);
            let on_clouds = air.landed(sun, look.sky.sun_size, look.clouds.height);
            let heavens = Heavens::new(
                (
                    placed.back().as_vec3(),
                    shining.color,
                    shining.illuminance / full,
                ),
                (
                    lights::toward(sun.elevation, sun.azimuth),
                    on_clouds.color,
                    on_clouds.illuminance / full,
                ),
                sky::night(daylight, sun, &look.sky),
                look,
            )
            .turned(
                area.lighting.sun.map_or(Mat3::IDENTITY, |noon| {
                    sky::sky_turn(daylight, noon.elevation, noon.azimuth)
                }),
                world
                    .get_resource::<VideoPreferences>()
                    .is_none_or(|video| video.twinkling),
            )
            .through(air.crossed_to(eye.map_or(ground, |eye| Meters(eye.y))))
            .covered(
                SkyWeather {
                    clouds,
                    water,
                    rain_lean: area
                        .climate
                        .rain
                        .as_ref()
                        .map_or(Vec2::ZERO, |rainfall| rainfall.lean(weather.wind.blowing())),
                },
                drifted,
                &look.clouds,
            );
            if let Some(dome) = scenery.sky {
                light_dome(world, dome, heavens);
            }
            if let Some(picture) = scenery.picture {
                light_dome(world, picture.face, heavens);
            }
            if let Some(rain) = scenery.rain {
                light_rain(world, rain, heavens);
            }
        }
    }
    let mut tone = area.lighting.grade.tone();
    if let Some(landed) = landed {
        tone.exposure.0 += sky::adaptation(daylight, landed.all(), look).0;
    }
    let rains = world.get_resource::<CameraRig>().map_or(0.0, |rig| {
        world.resource::<RainField>().rain_at(rig.target().xz())
    });
    if let Some(eye) = eye {
        if let Some(air) = scenery.air {
            let (volume, placed) = air_round(area, eye.xz(), moist, rains);
            hold_by(world, air, volume, same_air);
            hold(world, air, placed);
        }
        let mut banks: Vec<(&LaidFog, f32)> = area
            .fogs
            .iter()
            .map(|laid| {
                (
                    laid,
                    crate::systems::area::sky::share(world, laid.bank.when.as_ref()),
                )
            })
            .filter(|(_, share)| *share > 0.0)
            .collect();
        banks.sort_by(|(a, _), (b, _)| {
            a.bank
                .distance(eye.xz())
                .total_cmp(&b.bank.distance(eye.xz()))
        });
        let sight = sight_of(world, camera, area.heights.range().lowest);
        // A white face square to the full sun shines at this, which a colour of 1 stands for.
        let white = look.light.sun_lux / PI;
        let boxes: Vec<HazeBox> = banks
            .into_iter()
            .take(MAX_HAZE_BOXES)
            .map(|(laid, share)| laid.seen(moist, rains, share, white))
            .filter(|lying| sight.is_none_or(|sight| sight.crosses(lying.low, lying.high)))
            .collect();
        let picture = scenery
            .picture
            .and_then(|picture| world.get::<SkyPicture>(picture.camera))
            .map(|picture| picture.image.clone());
        let lights = fog_lights(landed, &boxes, eye);
        hold(world, camera, Haze::new(lights, picture.is_some(), boxes));
        if let Some(picture) = picture {
            hold(world, camera, HazeSky(picture));
        }
    }
    hold_by(world, camera, ambient, same_ambient);
    if let Some(grade) = world.get::<WorldPicture>(camera).map(|picture| picture.0) {
        hold(world, camera, camera::grading(tone, grade));
    }
    sky::show(world, camera, area.lighting.sky, farthest_seen(area));
    if let Some(picture) = scenery.picture {
        sky_picture::follow(world, picture.camera, camera);
    }
}

/// The farthest a camera in `area` sees anything of it: from one corner of its ground to the
/// other, over all its heights.
fn farthest_seen(area: &Area) -> Meters {
    let across = area.size.bounds().size;
    let range = area.heights.range();
    Meters(
        Vec3::new(
            across.width,
            across.height,
            range.highest.0 - range.lowest.0,
        )
        .length(),
    )
}

/// Where `camera`'s rays run over ground lying nowhere lower than `floor`, once it has a lens
/// and a place.
fn sight_of(world: &World, camera: Entity, floor: Meters) -> Option<HazeSight> {
    Some(HazeSight::of(
        world.get::<Camera>(camera)?.clip_from_view(),
        world.get::<GlobalTransform>(camera)?,
        floor.0,
    ))
}

/// Reaches the sun's shadows as far as `camera` sees the ground of `area`.
fn reach_shadows(world: &mut World, area: &Area, sun: Entity, camera: Entity) {
    let (Some(lens), Some(eye)) = (
        world.get::<Camera>(camera),
        world.get::<GlobalTransform>(camera),
    ) else {
        return;
    };
    let needed = lights::ShadowReach::seen(
        lens.clip_from_view(),
        eye,
        &area.heights,
        area.size.bounds(),
    );
    if let Some(mut reach) = world.get_mut::<lights::ShadowReach>(sun) {
        let next = reach.following(needed);
        reach.set_if_neq(next);
    }
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

/// The low air the sun's shafts show in round `eye` (x east, y south): in the bank of fog the
/// eye stands in, as far round as the fog leaves what is seen clear, as moist as `moist` while it
/// `rains` round the eye; none outside every bank.
fn air_round(area: &Area, eye: Vec2, moist: f32, rains: f32) -> (FogVolume, Transform) {
    let at = Pos::new(eye.x, eye.y);
    area.fogs
        .iter()
        .find(|laid| laid.bank.rect.contains(at))
        .and_then(|laid| {
            let bank = &laid.bank;
            let reach = (bank.sight_in(moist, rains).0 / SIGHT_LENGTHS).max(1.0);
            let round = Rect::new(
                Pos::new(eye.x - reach, eye.y - reach),
                Size::new(reach * 2.0, reach * 2.0),
            );
            bank.rect
                .intersection(&round)
                .map(|within| sky::air(within, laid.floor, bank.depth, bank.shafts.air(moist)))
        })
        .unwrap_or_else(|| sky::air(Rect::zero(), Meters(0.0), Meters(0.0), 0.0))
}

/// The sun and the moon as the fog round `eye` throws their light onward: each as it has
/// `landed`, less what the banks in `boxes` take of it on its way down to the eye.
fn fog_lights(landed: Option<SkyLights>, boxes: &[HazeBox], eye: Vec3) -> [HazeLight; 2] {
    let mut lights = [HazeLight::default(); 2];
    for (slot, light) in lights
        .iter_mut()
        .zip(landed.iter().flat_map(SkyLights::all))
    {
        let toward = lights::toward(light.elevation, light.azimuth);
        let crossed: f32 = boxes
            .iter()
            .map(|lying| lying.crossed(eye, eye + toward * BEYOND_FOG.0))
            .sum();
        *slot = HazeLight {
            toward,
            shed: light.color.to_linear().to_vec3() * light.illuminance * (-crossed).exp(),
        };
    }
    lights
}

/// How far toward a light the fog over a point is followed: past the top of any bank.
const BEYOND_FOG: Meters = Meters(100_000.0);

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
        look.params.get(name).copied().map(ShaderValue::from)
    });
    let material = SkyMaterial {
        site: SiteUniform::default(),
        params,
        weather: world.resource::<ShaderBoardImage>().image.clone(),
        heavens: Heavens::clear(world.resource::<Look>()),
        clouds: world.resource::<CloudSheet>().field(),
        shader,
        picture: false,
    };
    let material = world.resource_mut::<Assets<SkyMaterial>>().add(material);
    let dome = heavens::dome(&mut world.resource_mut::<Assets<Mesh>>(), material);
    Some(world.spawn((dome, ChildOf(root))).id())
}

/// The picture of the sky painted on `dome`, which a sky seen on `seen_on` is drawn into by the
/// dome's own shader.
fn picture_sky(
    world: &mut World,
    dome: Entity,
    seen_on: &RenderLayers,
    root: Entity,
) -> Option<PicturedSky> {
    let painted = world.get::<MeshMaterial3d<SkyMaterial>>(dome)?.0.clone();
    let mut materials = world.resource_mut::<Assets<SkyMaterial>>();
    let material = SkyMaterial {
        picture: true,
        ..materials.get(&painted)?.clone()
    };
    let material = materials.add(material);
    let layers = sky_picture::layers(seen_on);
    let camera = sky_picture::camera(&mut world.resource_mut::<Assets<Image>>(), layers.clone());
    let camera = world.spawn((camera, ChildOf(root))).id();
    let face = sky_picture::face(&mut world.resource_mut::<Assets<Mesh>>(), layers);
    let face = world
        .spawn((face, MeshMaterial3d(material), ChildOf(root)))
        .id();
    Some(PicturedSky { camera, face })
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
        rainfall.params.get(name).copied().map(ShaderValue::from)
    });
    let material = RainMaterial {
        site: SiteUniform::default(),
        params,
        weather: world.resource::<ShaderBoardImage>().image.clone(),
        heavens: Heavens::clear(world.resource::<Look>()),
        clouds: world.resource::<CloudSheet>().field(),
        shader,
    };
    let material = world.resource_mut::<Assets<RainMaterial>>().add(material);
    let shells = heavens::shells(
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

/// Has the light `entity` shine as `light` does above the air, its disc showing as `disc`.
fn shine(world: &mut World, entity: Entity, light: SunLight, disc: SunDisk) {
    if let Some(mut held) = world.get_mut::<DirectionalLight>(entity)
        && (held.color, held.illuminance) != (light.color, light.illuminance)
    {
        held.color = light.color;
        held.illuminance = light.illuminance;
    }
    hold_by(world, entity, disc, |a, b| {
        (a.angular_size, a.intensity) == (b.angular_size, b.intensity)
    });
}

/// The disc of the sun, which the air draws, or of the moon, which the sky's dome paints.
fn sky_disc(is_sun: bool, look: &SkyLook) -> SunDisk {
    if is_sun {
        lights::disc(look.sun_size, true)
    } else {
        lights::disc(look.moon_size, false)
    }
}

fn lights_at(area: &Area, weather: &Weather, daylight: Daylight, look: &Look) -> Option<SkyLights> {
    area.lighting.sun.map(|sun| {
        SkyLights::at(
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

/// The wind, cloud and rain `weather` makes under `climate` for what the client draws, lightning
/// lighting the world `flash` of the way to a strike close by.
pub fn shown_weather(climate: &Climate, weather: &Weather, flash: f32) -> ShownWeather {
    ShownWeather {
        wind: weather.wind.blowing(),
        clouds: weather.clouds,
        water: weather.water,
        rain: weather.rain,
        flash,
        ripples: climate.rain.as_ref().and_then(|rain| rain.ripples),
    }
}

pub fn grade(lighting: &Lighting) -> AreaGrade {
    let color = lighting.grade.color.color().to_linear();
    AreaGrade {
        color: Vec3::new(color.red, color.green, color.blue),
        strength: lighting.grade.strength,
    }
}

/// An area's ground as it is shaped: its heights, the terrains painted over it, the surface of its
/// water, and which of `waters` lies where.
#[derive(Clone, Copy)]
pub struct ShapedGround<'a> {
    pub heights: &'a Heightfield,
    pub paint: &'a PaintMap,
    pub water: &'a WaterSurface,
    pub water_map: &'a WaterMap,
    pub waters: &'a [WaterId],
}

impl<'a> ShapedGround<'a> {
    pub fn of(area: &'a Area) -> ShapedGround<'a> {
        ShapedGround {
            heights: &area.heights,
            paint: &area.paint,
            water: &area.water,
            water_map: &area.water_map,
            waters: &area.waters,
        }
    }
}

/// Lays `scenery`'s ground anew as `ground`, as a stroke shapes it.
pub fn lay_ground(world: &mut World, scenery: Scenery, ground: ShapedGround) {
    world.resource_scope(|world, mut streams: Mut<Streams>| {
        if let Some(streaming) = streams.0.get_mut(&scenery.root) {
            if ground.waters != streaming.waters {
                let content = streaming.content.clone();
                lay_waters(world, streaming, &content, ground.waters);
            }
            lay_streamed_ground(world, streaming, ground);
        }
    });
}

/// Lays the sheets and the looks of `waters`, read from `content`, in place of those of the
/// waters the scenery stood with.
fn lay_waters(world: &mut World, streaming: &mut Streaming, content: &Content, waters: &[WaterId]) {
    let looks = water::looks_image(&waters::looks(content, waters));
    let looks = world.resource_mut::<Assets<Image>>().add(looks);
    let lying = waters::sheets(
        world,
        content,
        &streaming.area,
        waters,
        streaming.mirrored.as_ref(),
    );
    if let Some(mut draw) = world.get_mut::<GroundDraw>(streaming.terrain) {
        draw.paint.waters = looks;
        draw.sheets = draw
            .sheets
            .iter()
            .filter(|sheet| matches!(sheet.over, SheetOver::Terrain(_)))
            .cloned()
            .chain(lying)
            .collect();
    }
    streaming.waters = waters.to_vec();
}

fn lay_streamed_ground(world: &mut World, streaming: &mut Streaming, ground: ShapedGround) {
    let size = streaming.heights.size();
    let changed: Vec<Rect<Meters>> = size
        .each_tile()
        .filter(|&at| {
            ground.heights.version(at) != streaming.heights.version(at)
                || ground.paint.version(at) != streaming.paint.version(at)
                || ground.water.version(at) != streaming.water.version(at)
                || ground.water_map.version(at) != streaming.water_map.version(at)
        })
        .map(|at| at.rect(size).inflate(EDGE_REACH.0, EDGE_REACH.0))
        .collect();
    if changed.is_empty() {
        return;
    }
    streaming.heights = ground.heights.clone();
    streaming.paint = ground.paint.clone();
    streaming.water = ground.water.clone();
    streaming.water_map = ground.water_map.clone();
    if let Some(mut atlas) = world.get_mut::<TerrainAtlas>(streaming.terrain) {
        atlas.lay(
            ground.heights,
            ground.paint,
            ground.water,
            ground.water_map,
            detail_slots(),
        );
    }
    streaming
        .instances
        .ground_changed(streaming.area.size.bounds(), &changed);
    streaming.grass.ground_changed(&changed);
    streaming.whole = false;
}

/// How far past a tile of ground what stands on it may read the ground, so a change to the tile
/// changes it too: the samples a slope is read across, and the reach of a copy or a blade over
/// the edge of the cell it is laid out in.
const EDGE_REACH: Meters = Meters(2.0);

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
    let waters = waters::looks(content, &area.waters);
    let weather = world.resource::<ShaderBoardImage>().image.clone();
    let (paint, pictures) = world.resource_scope(|world, mut images: Mut<Assets<Image>>| {
        let assets = world.resource::<AssetServer>();
        terrain::ground_paint(
            &layers,
            &waters,
            &look.materials,
            weather,
            assets,
            &mut images,
        )
    });
    let mut atlas = TerrainAtlas::default();
    atlas.lay(
        &area.heights,
        &area.paint,
        &area.water,
        &area.water_map,
        detail_slots(),
    );
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
                silt: def.silt,
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
    let lamp = Lamp {
        color: light.color.color(),
        intensity: light.intensity,
        radius: light.radius,
        flicker: light.flicker,
        shadows_from: None,
        aura: None,
    }
    .spawn(world, rng.rand_float() * std::f32::consts::TAU);
    world.entity_mut(lamp).insert((
        Transform::from_translation(world_point(light.at, ground + light.height)),
        ChildOf(root),
    ));
    if let Some(when) = &light.when {
        world
            .entity_mut(lamp)
            .insert((SkyLit(when.clone()), Opacity(0.0), Visibility::Hidden));
    }
    lamp
}

/// A stood light that shines as much as its condition of the sky says.
#[derive(Component)]
struct SkyLit(SkyCondition);

/// Lets every light that follows the sky shine as much as the sky has it now. One with none of
/// it is not drawn at all.
fn light_by_sky(now: Res<SkyNow>, mut lights: Query<(&SkyLit, &mut Opacity, &mut Visibility)>) {
    for (lit, mut opacity, mut visibility) in &mut lights {
        let share = now.share(Some(&lit.0));
        opacity.set_if_neq(Opacity(share));
        let shown = if share > 0.0 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        visibility.set_if_neq(shown);
    }
}

/// Plans what an area's scenery stands for each sight, its lights filed off the frame.
fn planner_of(area: &Arc<Area>) -> Making<Arc<Planner>> {
    let area = area.clone();
    Making::Task(AsyncComputeTaskPool::get().spawn(async move {
        let mut lights = SpatialIndex::new(PLACEMENT_CELL);
        lights.refile_spots(area.lights.iter().map(|light| light.at));
        let mut acting = SpatialIndex::new(PLACEMENT_CELL);
        acting.refile_spots(area.acting.iter().map(|&index| area.props[index].at));
        Arc::new(Planner {
            lights,
            acting,
            area,
        })
    }))
}

/// Whether the tile at `at` holds the same values in `standing` as in `new`: the very same tile,
/// both blank, or both as read from the same file holding the same, by `hashes`, the files of the
/// contents each was read from.
fn tile_alike<T: GroundValue>(
    standing: &GroundGrid<T>,
    new: &GroundGrid<T>,
    at: TileAt,
    (was, now): (&BTreeMap<String, FileHash>, &BTreeMap<String, FileHash>),
) -> bool {
    let version = standing.version(at);
    if version != TileVersion::default() && version == new.version(at) {
        return true;
    }
    if standing.is_blank_at(at) && new.is_blank_at(at) {
        return true;
    }
    match (standing.stored_at(at), new.stored_at(at)) {
        (Some(a), Some(b)) => a == b && was.get(a).is_some_and(|hash| now.get(b) == Some(hash)),
        _ => false,
    }
}

/// Whether two contents name the same files of `Row` by the same ids.
fn same_paths<Row: ContentFile>(a: &Content, b: &Content) -> bool {
    a.table::<Row>().len() == b.table::<Row>().len()
        && a.ids::<Row>().all(|id| id.path(a) == id.path(b))
}

/// Whether two of an area's prop sources stand alike.
fn same_source(a: &AreaModule, b: &AreaModule) -> bool {
    match (a, b) {
        (AreaModule::Prop(a), AreaModule::Prop(b)) => a == b,
        (AreaModule::Scatter(a), AreaModule::Scatter(b)) => a == b,
        _ => false,
    }
}

/// The ground a prop source stands its props over, a little wider for a prop on a chunk's edge.
fn stretch_of(source: &AreaModule) -> Option<Rect<Meters>> {
    match source {
        AreaModule::Prop(prop) => Some(Rect::new(prop.at, Size::new(0.0, 0.0))),
        AreaModule::Scatter(scatter) => Some(scatter.region),
        _ => None,
    }
    .map(|stretch| stretch.inflate(EDGE_REACH.0, EDGE_REACH.0))
}
