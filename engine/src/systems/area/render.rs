//! The area on view, drawn: its terrain as one coloured mesh, its props as scenes on the ground,
//! its sun, ambient light, point lights and grade. Set up once per area entered and torn down when
//! the view leaves it.

use bevy::light::{AmbientLight, NotShadowCaster, Skybox};
use bevy::prelude::*;

use super::ground::CoverGeometry;
use super::{AreaId, AreaObject, AreaTag, Lighting, PointLightDef, PropPlacement};
use crate::core::content::Content;
use crate::core::geometry::Radians;
use crate::core::geometry::{PlacedShape, Shape};
use crate::core::math::Rng;
use crate::core::math::{Pos, Rect};
use crate::core::render::present::AreaGrade;
use crate::core::render::transition::WorldViewSystems;
use crate::core::render::world::cover;
use crate::core::render::world::fade::Fades;
use crate::core::render::world::lights::{self, AMBIENT_NITS, Flicker};
use crate::core::render::world::sky::{self, Daylight, SkyColors};
use crate::core::render::world::terrain;
use crate::core::render::world::toon::{ToonMaterial, toon_of};
use crate::core::render::{GroundHeights, WorldCamera, ground_height, world_point};
use crate::core::terrain::MATERIALS;
use crate::core::time::Seconds;
use crate::core::units::Meters;
use crate::core::vfx::cover::{self as cover_vfx, Climate, CoverMaterial};
use crate::core::vfx::render::{self as vfx_render, ParamsUniform, Run, SiteExtent, SiteFrame};
use crate::core::vfx::shader::Stage;
use crate::core::vfx::{ParamFill, VfxSite};
use crate::systems::player::session::MyCharacter;
use crate::systems::screen::ClientScreen;

pub struct AreaRenderPlugin;

impl Plugin for AreaRenderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ShownArea>()
            .init_resource::<Daylight>()
            .init_resource::<LitAt>()
            .add_systems(
                Update,
                (
                    super::weather::tell_time.run_if(resource_exists::<Content>),
                    show_area,
                    follow_daylight,
                    place_objects,
                )
                    .chain()
                    .in_set(WorldViewSystems)
                    .run_if(in_state(ClientScreen::Area)),
            )
            .add_systems(OnExit(ClientScreen::Area), leave_area);
    }
}

/// The area the view is set up for.
#[derive(Resource, Default)]
struct ShownArea(Option<AreaId>);

/// The area whose scenery stands in the view.
pub fn shown(world: &World) -> Option<AreaId> {
    world.resource::<ShownArea>().0
}

/// What the view spawned for the shown area, gone with it.
#[derive(Component)]
struct Scenery;

/// The shown area's sun, turned as the day goes.
#[derive(Component)]
struct AreaSun;

/// The step of the day the shown area was last lit for.
#[derive(Resource, Default)]
struct LitAt(Option<u32>);

fn show_area(world: &mut World) {
    let Some(area_id) = world
        .resource::<MyCharacter>()
        .0
        .and_then(|seen| world.get::<AreaTag>(seen))
        .map(|tag| tag.area)
    else {
        return;
    };
    if shown(world) == Some(area_id) {
        return;
    }
    tear_down(world);
    let content = world.resource::<Content>().clone();
    let area = super::shared(&content, area_id).clone();
    world.insert_resource(GroundHeights {
        heights: area.heights.clone(),
        bounds: area.terrain.size.bounds(),
    });
    world.insert_resource(grade_of(&area.lighting));
    let (sin, cos) = area.weather.wind.direction.radians().0.sin_cos();
    world.insert_resource(Climate {
        wind: Vec2::new(sin, -cos) * area.weather.wind.strength,
        overcast: area.weather.overcast,
    });
    if let Some(sun) = &area.lighting.sun {
        world.spawn((
            lights::sun(sun.color.color(), 0.0, sun.elevation, sun.azimuth),
            AreaSun,
            Scenery,
        ));
    }
    light_area(world, &area);
    spawn_terrain(world, &content, &area);
    spawn_covers(world, &content, &area);
    for placed in &area.props {
        spawn_prop(world, &content, placed);
    }
    let mut rng = Rng::from_entropy();
    for light in &area.lights {
        spawn_light(world, light, &mut rng);
    }
    world.resource_mut::<ShownArea>().0 = Some(area_id);
}

/// Lights the shown area for the hour, whenever the day has moved on a step.
fn follow_daylight(world: &mut World) {
    let Some(area_id) = shown(world) else {
        return;
    };
    let step = world.resource::<Daylight>().step();
    if world.resource::<LitAt>().0 == Some(step) {
        return;
    }
    let content = world.resource::<Content>().clone();
    let area = super::shared(&content, area_id).clone();
    light_area(world, &area);
}

/// Sets the area's sun, ambient light and sky for the daylight of the moment, under its weather.
fn light_area(world: &mut World, area: &super::Area) {
    let daylight = *world.resource::<Daylight>();
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
    if let Some(sun) = sun
        && let Ok((mut light, mut transform)) = world
            .query_filtered::<(&mut DirectionalLight, &mut Transform), With<AreaSun>>()
            .single_mut(world)
    {
        light.color = sun.color;
        light.illuminance = sun.illuminance;
        *transform = lights::sun_transform(sun.elevation, sun.azimuth);
    }
    let Ok(camera) = world
        .query_filtered::<Entity, With<WorldCamera>>()
        .single(world)
    else {
        return;
    };
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
    world.entity_mut(camera).insert((
        ambient,
        Skybox {
            image: Some(image),
            brightness: sky::SKY_NITS,
            ..default()
        },
    ));
    world.resource_mut::<LitAt>().0 = Some(daylight.step());
}

/// Gives every object of the shown world a place on the ground and the extent of its shape, so
/// what plays on it finds it where the area puts it.
fn place_objects(world: &mut World) {
    let objects: Vec<(Entity, Pos<Meters>, Rect<Meters>)> = world
        .query_filtered::<(Entity, &AreaObject), Without<SiteExtent>>()
        .iter(world)
        .map(|(entity, object)| (entity, object.shape.center(), object.shape.bounds()))
        .collect();
    for (entity, center, bounds) in objects {
        let ground = ground_height(world, center);
        world.entity_mut(entity).insert((
            Transform::from_translation(world_point(center, ground)),
            Visibility::default(),
            SiteExtent {
                footprint: Vec2::new(bounds.size.width.max(1.0), bounds.size.height.max(1.0)),
                height: 1.0,
            },
        ));
    }
}

fn leave_area(world: &mut World) {
    tear_down(world);
    world.resource_mut::<ShownArea>().0 = None;
}

fn tear_down(world: &mut World) {
    world.resource_mut::<LitAt>().0 = None;
    let scenery: Vec<Entity> = world
        .query_filtered::<Entity, With<Scenery>>()
        .iter(world)
        .collect();
    for entity in scenery {
        world.despawn(entity);
    }
}

fn grade_of(lighting: &Lighting) -> AreaGrade {
    let color = lighting.grade.color.color().to_linear();
    AreaGrade {
        color: Vec3::new(color.red, color.green, color.blue),
        strength: lighting.grade.strength,
    }
}

fn spawn_terrain(world: &mut World, content: &Content, area: &super::Area) {
    let palette: [[f32; 4]; MATERIALS] = area.terrain.materials.map(|ground| {
        let color = ground.get(content).color.color().to_linear();
        [color.red, color.green, color.blue, 1.0]
    });
    let mesh = terrain::mesh(&area.heights, |column, row| {
        let weights = area.paint.weights(column, row);
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
    });
    let mesh = world.resource_mut::<Assets<Mesh>>().add(mesh);
    let mut toon = toon_of(StandardMaterial::default());
    toon.extension.style.flat = 0.0;
    let material = world.resource_mut::<Assets<ToonMaterial>>().add(toon);
    world.spawn((
        Mesh3d(mesh),
        MeshMaterial3d(material),
        Transform::default(),
        Scenery,
    ));
}

/// Each ground material with a cover gets it laid over the whole area, a mesh per chunk so the
/// camera culls what it does not see, scattered alike on every client, drawn through the ground's
/// cover shader with the area as its site. A strewn prop's chunks wait for the prop to load.
fn spawn_covers(world: &mut World, content: &Content, area: &super::Area) {
    let (columns, rows) = area.terrain.size.paint_pixels();
    let bounds = area.terrain.size.bounds();
    let site = VfxSite::Area(PlacedShape {
        at: Pos::new(
            bounds.origin.x + bounds.size.width / 2.0,
            bounds.origin.y + bounds.size.height / 2.0,
        ),
        facing: Radians::default(),
        shape: Shape::Rect {
            width: Meters(bounds.size.width),
            height: Meters(bounds.size.height),
        },
    });
    for (slot, ground) in area.terrain.materials.iter().enumerate() {
        let ground = ground.get(content);
        let Some(cover_def) = &ground.cover else {
            continue;
        };
        let Some(shader) = vfx_render::compiled_shader(world, cover_def.shader) else {
            continue;
        };
        let Some(frame) = SiteFrame::of(world, &site, Radians::default()) else {
            continue;
        };
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
        let alpha = match cover_def.geometry {
            CoverGeometry::Strewn { .. } => AlphaMode::Mask(0.5),
            CoverGeometry::Sheet { .. } | CoverGeometry::Blades { .. } => AlphaMode::Blend,
        };
        let material = world
            .resource_mut::<Assets<CoverMaterial>>()
            .add(cover_vfx::material(
                ground.color.color(),
                frame.uniform(run, Stage::Cover),
                params,
                shader,
                alpha,
            ));
        let share = |column: u32, row: u32| {
            let weights = area.paint.weights(column, row);
            let total: f32 = weights.iter().map(|weight| f32::from(*weight)).sum();
            if total <= 0.0 {
                0.0
            } else {
                f32::from(weights[slot]) / total
            }
        };
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
                let seed = u64::from(chunk_row) << 24 | u64::from(chunk_column) << 4 | slot as u64;
                let laid = match &cover_def.geometry {
                    CoverGeometry::Sheet { level } => {
                        cover::sheet(&patch, level.map(|level| level.0))
                            .map(|mesh| lay(world, mesh))
                    }
                    CoverGeometry::Blades {
                        per_square_meter,
                        height,
                        width,
                    } => {
                        let blades = cover::Blades {
                            per_square_meter: *per_square_meter,
                            height: height.0,
                            width: width.0,
                        };
                        cover::blades(&patch, &blades, seed).map(|mesh| lay(world, mesh))
                    }
                    CoverGeometry::Strewn {
                        prop,
                        per_square_meter,
                        scale,
                    } => {
                        let strew = cover::Strew {
                            per_square_meter: *per_square_meter,
                            scale: *scale,
                        };
                        let places = cover::strew_places(&patch, &strew, seed);
                        (!places.is_empty()).then(|| Laid::Strewn {
                            model: prop.get(content).model.path(),
                            places,
                        })
                    }
                };
                let Some(laid) = laid else {
                    continue;
                };
                let mut chunk = world.spawn((
                    MeshMaterial3d(material.clone()),
                    Transform::default(),
                    Visibility::default(),
                    NotShadowCaster,
                    Scenery,
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

fn spawn_prop(world: &mut World, content: &Content, placed: &PropPlacement) {
    let def = placed.prop.get(content);
    let ground = world.resource::<GroundHeights>().height_at(placed.at);
    let scene = world
        .resource::<AssetServer>()
        .load(GltfAssetLabel::Scene(0).from_asset(def.model.path()));
    let mut prop = world.spawn((
        WorldAssetRoot(scene),
        Transform {
            translation: world_point(placed.at, ground),
            rotation: Quat::from_rotation_y(-placed.yaw.radians().0),
            scale: Vec3::splat(placed.scale),
        },
        Scenery,
    ));
    if def.fade {
        prop.insert(Fades::default());
    }
    if !def.shadow {
        prop.insert(NotShadowCaster);
    }
}

fn spawn_light(world: &mut World, light: &PointLightDef, rng: &mut Rng) {
    let ground = world.resource::<GroundHeights>().height_at(light.at);
    let point = lights::point(light.color.color(), light.intensity, light.radius.0);
    let full = point.intensity;
    let mut spawned = world.spawn((
        point,
        Transform::from_translation(world_point(light.at, ground + light.height)),
        Scenery,
    ));
    if light.flicker > 0.0 {
        spawned.insert(Flicker {
            amount: light.flicker,
            full,
            phase: rng.rand_float() * std::f32::consts::TAU,
        });
    }
}
