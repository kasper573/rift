//! What stands in a world for an area: its ground as one coloured mesh, the covers its terrains
//! grow, its props as scenes on the ground, its point lights, its sun, the sky its sun lights and
//! the air over it, every one a child of one root. The client stands the area the player is in;
//! the editor stands the area it edits, on render layers of its own, and both light it alike for
//! the hour.

use bevy::app::Propagate;
use bevy::camera::primitives::MeshAabb;
use bevy::camera::visibility::RenderLayers;
use bevy::light::NotShadowCaster;
use bevy::light::atmosphere::ScatteringMedium;
use bevy::prelude::*;

use super::lighting::DomeLook;
use super::prop::PropId;
use super::terrain::{CoverPiece, PieceShape};
use super::{Area, Lighting, PointLightDef, PropPlacement};
use crate::core::content::media::GltfInfos;
use crate::core::content::{Content, FileHash, FileHashes};
use crate::core::geometry::Radians;
use crate::core::geometry::{PlacedShape, Shape};
use crate::core::ground::{Heightfield, Paint};
use crate::core::math::Pos;
use crate::core::math::Rng;
use crate::core::render::present::AreaGrade;
use crate::core::render::world::clouds::{self, CloudSheet};
use crate::core::render::world::cover;
use crate::core::render::world::fade::Fades;
use crate::core::render::world::lights::{self, AMBIENT_NITS, Flicker, SUN_LUX};
use crate::core::render::world::material::{MaterialLook, MaterialLooks};
use crate::core::render::world::sky::{self, Daylight, SunLight};
use crate::core::render::world::terrain::{self, GroundLayer, GroundMaterial};
use crate::core::render::{camera, world_point};
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

/// Stands `area` in the world on `layers`, with its terrain, covers, props, lights, sun, sky and
/// air.
pub fn spawn(world: &mut World, content: &Content, area: &Area, layers: RenderLayers) -> Scenery {
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
    spawn_covers(world, content, area, root);
    for placed in &area.props {
        spawn_prop(world, content, &area.heights, placed, root);
    }
    let mut rng = Rng::from_entropy();
    for light in &area.lights {
        spawn_light(world, &area.heights, light, &mut rng, root);
    }
    Scenery {
        root,
        terrain,
        sun,
        sky: dome,
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
    let mesh = world
        .resource_mut::<Assets<Mesh>>()
        .add(terrain::mesh(heights));
    let Ok(mut ground) = world.get_entity_mut(scenery.terrain) else {
        return;
    };
    ground.insert(Mesh3d(mesh));
    let Some(material) = ground
        .get::<MeshMaterial3d<GroundMaterial>>()
        .map(|material| material.0.clone())
    else {
        return;
    };
    world.resource_scope(|world, mut images: Mut<Assets<Image>>| {
        let mut materials = world.resource_mut::<Assets<GroundMaterial>>();
        terrain::repaint(&material, paint, &mut materials, &mut images);
    });
}

/// The ground on the area's heights, painted with its terrains.
fn spawn_terrain(world: &mut World, content: &Content, area: &Area, root: Entity) -> Entity {
    let mesh = world
        .resource_mut::<Assets<Mesh>>()
        .add(terrain::mesh(&area.heights));
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
            Mesh3d(mesh),
            MeshMaterial3d(material),
            pictures,
            Transform::default(),
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

/// Each terrain with a cover gets it laid over the whole area, a mesh per chunk of a sheet or
/// blades so the camera culls what it does not see, scattered alike on every client from the
/// cover's seed, drawn through the terrain's cover shader with the area as its site. A prop
/// piece's copies wait for the prop to load.
fn spawn_covers(world: &mut World, content: &Content, area: &Area, root: Entity) {
    let (columns, rows) = area.paint.pixels();
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
        let share = |column: u32, row: u32| {
            if usize::from(area.paint.index(column, row)) == slot {
                1.0
            } else {
                0.0
            }
        };
        let ground_color = ground.color.color().to_linear();
        let layouts: Vec<(Option<&CoverPiece>, cover::Spread)> = std::iter::once((
            None,
            cover::Spread {
                piece: 0,
                per_square_meter: 0.0,
                size: (1.0, 1.0),
                color: (ground_color, ground_color),
            },
        ))
        .filter(|_| cover_def.sheet.is_some())
        .chain(cover_def.pieces.iter().enumerate().map(|(index, piece)| {
            (
                Some(piece),
                cover::Spread {
                    piece: index as u32,
                    per_square_meter: piece.per_square_meter(piece.shape.base_area(prop_bounds)),
                    size: (piece.size.min, piece.size.max),
                    color: (
                        piece.color.from.color().to_linear(),
                        piece.color.to.color().to_linear(),
                    ),
                },
            )
        }))
        .collect();
        for (piece, spread) in &layouts {
            let mut chunks = Vec::new();
            for chunk_row in 0..rows.div_ceil(cover::CHUNK_CELLS) {
                for chunk_column in 0..columns.div_ceil(cover::CHUNK_CELLS) {
                    let first = (
                        chunk_column * cover::CHUNK_CELLS,
                        chunk_row * cover::CHUNK_CELLS,
                    );
                    let patch = cover::Patch {
                        heights: &area.heights,
                        first,
                        columns: cover::CHUNK_CELLS.min(columns - first.0),
                        rows: cover::CHUNK_CELLS.min(rows - first.1),
                        share: &share,
                    };
                    let seed = chunk_seed(
                        cover_def.seed,
                        [
                            slot as u64,
                            u64::from(spread.piece),
                            u64::from(chunk_row),
                            u64::from(chunk_column),
                        ],
                    );
                    let mesh = match piece.map(|piece| &piece.shape) {
                        None => cover::sheet(
                            &patch,
                            cover_def
                                .sheet
                                .and_then(|sheet| sheet.level)
                                .map(|level| level.0),
                            ground_color,
                        ),
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
                            let places = cover::strew_places(&patch, spread, seed);
                            if !places.is_empty() {
                                chunks.push(places);
                            }
                            None
                        }
                    };
                    if let Some((bounds, mesh)) =
                        mesh.and_then(|mesh| Some((mesh.get_aabb()?, mesh)))
                    {
                        let mesh = world.resource_mut::<Assets<Mesh>>().add(mesh);
                        world.spawn((
                            Mesh3d(mesh),
                            MeshMaterial3d(blended.clone()),
                            bounds,
                            NotShadowCaster,
                            ChildOf(root),
                        ));
                    }
                }
            }
            let Some(PieceShape::Prop { prop }) = piece.map(|piece| &piece.shape) else {
                continue;
            };
            if chunks.is_empty() {
                continue;
            }
            let gltf = world
                .resource::<AssetServer>()
                .load(prop.get(content).model.path());
            let material = material(world, Some(spread));
            world.spawn((
                cover::Strewing {
                    gltf,
                    chunks,
                    material,
                },
                Transform::default(),
                Visibility::default(),
                ChildOf(root),
            ));
        }
    }
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
) {
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
}

fn spawn_light(
    world: &mut World,
    heights: &Heightfield,
    light: &PointLightDef,
    rng: &mut Rng,
    root: Entity,
) {
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
}
