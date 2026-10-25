//! What stands in a world for an area: its terrain as one coloured mesh, the covers its grounds
//! grow, its props as scenes on the ground, its point lights and its sun, every one a child of one
//! root. The client stands the area the player is in; the editor stands the area it edits, on
//! render layers of its own, and both light it alike for the hour.

use bevy::app::Propagate;
use bevy::camera::visibility::RenderLayers;
use bevy::light::{AmbientLight, NotShadowCaster, Skybox};
use bevy::prelude::*;

use super::ground::{CoverPiece, PieceShape};
use super::prop::PropId;
use super::{Area, Lighting, PointLightDef, PropPlacement};
use crate::core::content::Content;
use crate::core::content::media::GltfInfos;
use crate::core::geometry::Radians;
use crate::core::geometry::{PlacedShape, Shape};
use crate::core::math::Pos;
use crate::core::math::Rng;
use crate::core::render::present::AreaGrade;
use crate::core::render::world::cover;
use crate::core::render::world::fade::Fades;
use crate::core::render::world::lights::{self, AMBIENT_NITS, Flicker};
use crate::core::render::world::sky::{self, Daylight, SkyColors};
use crate::core::render::world::terrain;
use crate::core::render::world::toon::{ToonMaterial, toon_of};
use crate::core::render::world_point;
use crate::core::terrain::{Heightfield, MATERIALS, Paint};
use crate::core::time::Seconds;
use crate::core::units::Meters;
use crate::core::vfx::ParamFill;
use crate::core::vfx::cover::{self as cover_vfx, Climate, CoverMaterial};
use crate::core::vfx::render::{self as vfx_render, ParamsUniform, Run, SiteFrame};
use crate::core::vfx::shader::Stage;

/// An area's scenery as it stands in a world: everything hangs under `root`, the terrain mesh is
/// `terrain`, and `sun` is the directional light when the area has one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scenery {
    pub root: Entity,
    pub terrain: Entity,
    pub sun: Option<Entity>,
}

/// Stands `area` in the world on `layers`, with its terrain, covers, props, lights and sun.
pub fn spawn(world: &mut World, content: &Content, area: &Area, layers: RenderLayers) -> Scenery {
    let root = world
        .spawn((
            Transform::default(),
            Visibility::default(),
            Propagate(layers),
        ))
        .id();
    let sun = area.lighting.sun.map(|sun| {
        world
            .spawn((
                lights::sun(sun.color.color(), 0.0, sun.elevation, sun.azimuth),
                ChildOf(root),
            ))
            .id()
    });
    let terrain = spawn_terrain(world, content, area, root);
    spawn_covers(world, content, area, root);
    for placed in &area.props {
        spawn_prop(world, content, &area.heights, placed, root);
    }
    let mut rng = Rng::from_entropy();
    for light in &area.lights {
        spawn_light(world, &area.heights, light, &mut rng, root);
    }
    Scenery { root, terrain, sun }
}

/// Whether every cover shader `area`'s grounds draw with is compiled, so the scenery can stand
/// whole: the shaders follow the content into the world a frame behind it.
pub fn shaders_ready(world: &World, content: &Content, area: &Area) -> bool {
    area.terrain.materials.iter().all(|ground| {
        ground
            .get(content)
            .cover
            .as_ref()
            .is_none_or(|cover| vfx_render::compiled_shader(world, cover.shader).is_some())
    })
}

pub fn despawn(world: &mut World, scenery: Scenery) {
    if world.get_entity(scenery.root).is_ok() {
        world.despawn(scenery.root);
    }
}

/// Lights `area` for the hour under its weather: its sun turned and coloured, and the ambient light
/// and sky put on `camera`.
pub fn light(world: &mut World, area: &Area, daylight: Daylight, scenery: Scenery, camera: Entity) {
    let overcast = area.weather.overcast;
    let sun = area.lighting.sun.map(|sun| {
        sky::sun_at(
            daylight,
            sun.elevation,
            sun.azimuth,
            sun.color.color(),
            overcast,
        )
    });
    let elevation = sun.map(|sun| sun.elevation);
    let authored = area.lighting.sky();
    let colors = sky::sky_at(
        SkyColors {
            zenith: authored.zenith.color(),
            horizon: authored.horizon.color(),
        },
        daylight,
        elevation,
        overcast,
    );
    let ambient = AmbientLight {
        color: area.lighting.ambient.color(),
        brightness: AMBIENT_NITS * sky::ambient_at(daylight, elevation, overcast),
        ..default()
    };
    if let (Some(sun), Some(entity)) = (sun, scenery.sun) {
        if let Some(mut light) = world.get_mut::<DirectionalLight>(entity) {
            light.color = sun.color;
            light.illuminance = sun.illuminance;
        }
        if let Some(mut transform) = world.get_mut::<Transform>(entity) {
            *transform = lights::sun_transform(sun.elevation, sun.azimuth);
        }
    }
    let shown_sky = world
        .get::<Skybox>(camera)
        .and_then(|skybox| skybox.image.clone());
    let cubemap = sky::cubemap(colors);
    let mut images = world.resource_mut::<Assets<Image>>();
    let image = match shown_sky {
        Some(handle) if images.contains(handle.id()) => {
            if let Some(mut shown) = images.get_mut(handle.id()) {
                *shown = cubemap;
            }
            handle
        }
        _ => images.add(cubemap),
    };
    if let Ok(mut camera) = world.get_entity_mut(camera) {
        camera.insert((
            ambient,
            Skybox {
                image: Some(image),
                brightness: sky::SKY_NITS,
                ..default()
            },
        ));
    }
}

/// The wind and cloud the area's covers move under.
pub fn climate(area: &Area) -> Climate {
    let (sin, cos) = area.weather.wind.direction.radians().0.sin_cos();
    Climate {
        wind: Vec2::new(sin, -cos) * area.weather.wind.strength,
        overcast: area.weather.overcast,
    }
}

pub fn grade(lighting: &Lighting) -> AreaGrade {
    let color = lighting.grade.color.color().to_linear();
    AreaGrade {
        color: Vec3::new(color.red, color.green, color.blue),
        strength: lighting.grade.strength,
    }
}

/// The terrain's mesh: `heights` coloured by `paint` with the area's materials.
pub fn terrain_mesh(content: &Content, area: &Area, heights: &Heightfield, paint: &Paint) -> Mesh {
    let palette: [[f32; 4]; MATERIALS] = area.terrain.materials.map(|ground| {
        let color = ground.get(content).color.color().to_linear();
        [color.red, color.green, color.blue, 1.0]
    });
    terrain::mesh(heights, |column, row| {
        let weights = paint.weights(column, row);
        let total: f32 = weights
            .iter()
            .map(|weight| f32::from(*weight))
            .sum::<f32>()
            .max(1.0);
        let mut color = [0.0; 4];
        for (material, weight) in weights.iter().enumerate() {
            let share = f32::from(*weight) / total;
            for channel in 0..3 {
                color[channel] += palette[material][channel] * share;
            }
        }
        color[3] = 1.0;
        color
    })
}

fn spawn_terrain(world: &mut World, content: &Content, area: &Area, root: Entity) -> Entity {
    let mesh = terrain_mesh(content, area, &area.heights, &area.paint);
    let mesh = world.resource_mut::<Assets<Mesh>>().add(mesh);
    let mut toon = toon_of(StandardMaterial::default());
    toon.extension.style.flat = 0.0;
    let material = world.resource_mut::<Assets<ToonMaterial>>().add(toon);
    world
        .spawn((
            Mesh3d(mesh),
            MeshMaterial3d(material),
            Transform::default(),
            ChildOf(root),
        ))
        .id()
}

/// Each ground material with a cover gets it laid over the whole area, a mesh per chunk and piece
/// so the camera culls what it does not see, scattered alike on every client from the cover's
/// seed, drawn through the ground's cover shader with the area as its site. A prop piece's
/// chunks wait for the prop to load.
fn spawn_covers(world: &mut World, content: &Content, area: &Area, root: Entity) {
    let (columns, rows) = area.terrain.size.paint_pixels();
    let bounds = area.terrain.size.bounds();
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
    for (slot, ground) in area.terrain.materials.iter().enumerate() {
        let ground = ground.get(content);
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
        let material = |world: &mut World, alpha| {
            world
                .resource_mut::<Assets<CoverMaterial>>()
                .add(cover_vfx::material(
                    frame.uniform(run, Stage::Cover),
                    params,
                    shader.clone(),
                    alpha,
                ))
        };
        let blended = material(world, AlphaMode::Blend);
        let masked = material(world, AlphaMode::Mask(0.5));
        let share = |column: u32, row: u32| {
            let weights = area.paint.weights(column, row);
            let total: f32 = weights.iter().map(|weight| f32::from(*weight)).sum();
            if total <= 0.0 {
                0.0
            } else {
                f32::from(weights[slot]) / total
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
                for (piece, spread) in &layouts {
                    let seed = chunk_seed(
                        cover_def.seed,
                        [
                            slot as u64,
                            u64::from(spread.piece),
                            u64::from(chunk_row),
                            u64::from(chunk_column),
                        ],
                    );
                    let laid = match piece.map(|piece| &piece.shape) {
                        None => cover::sheet(
                            &patch,
                            cover_def
                                .sheet
                                .and_then(|sheet| sheet.level)
                                .map(|level| level.0),
                            ground_color,
                        )
                        .map(|mesh| lay(world, mesh)),
                        Some(PieceShape::Blade { width, height }) => cover::blades(
                            &patch,
                            &cover::BladeShape {
                                width: width.0,
                                height: height.0,
                            },
                            spread,
                            seed,
                        )
                        .map(|mesh| lay(world, mesh)),
                        Some(PieceShape::Prop { prop }) => {
                            let places = cover::strew_places(&patch, spread, seed);
                            (!places.is_empty()).then(|| Laid::Strewn {
                                model: prop.get(content).model.path(),
                                places,
                            })
                        }
                    };
                    let Some(laid) = laid else {
                        continue;
                    };
                    let material = match laid {
                        Laid::Mesh(_) => blended.clone(),
                        Laid::Strewn { .. } => masked.clone(),
                    };
                    let mut chunk = world.spawn((
                        MeshMaterial3d(material),
                        Transform::default(),
                        Visibility::default(),
                        NotShadowCaster,
                        ChildOf(root),
                    ));
                    match laid {
                        Laid::Mesh(mesh) => {
                            chunk.insert(Mesh3d(mesh));
                        }
                        Laid::Strewn { model, places } => {
                            let gltf = chunk.world().resource::<AssetServer>().load(model);
                            chunk.insert(cover::Strewing { gltf, places });
                        }
                    }
                }
            }
        }
    }
}

/// A chunk of cover as it is laid: a mesh built from the paint, or places for a prop to stand once
/// its model loads.
enum Laid {
    Mesh(Handle<Mesh>),
    Strewn {
        model: String,
        places: Vec<cover::Place>,
    },
}

fn lay(world: &mut World, mesh: Mesh) -> Laid {
    Laid::Mesh(world.resource_mut::<Assets<Mesh>>().add(mesh))
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
        translation: world_point(placed.at, heights.height_at(placed.at)),
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
    let mut prop = world.spawn((
        WorldAssetRoot(scene),
        prop_transform(heights, placed),
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
