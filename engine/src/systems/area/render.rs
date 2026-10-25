//! The area on view, drawn: its terrain as one coloured mesh, its props as scenes on the ground,
//! its sun, ambient light, point lights and grade. Set up once per area entered and torn down when
//! the view leaves it.

use bevy::light::{AmbientLight, NotShadowCaster};
use bevy::prelude::*;

use super::{AreaId, AreaObject, AreaTag, Lighting, PointLightDef, PropPlacement};
use crate::core::content::Content;
use crate::core::math::Rng;
use crate::core::math::{Pos, Rect};
use crate::core::render::present::AreaGrade;
use crate::core::render::transition::WorldViewSystems;
use crate::core::render::world::fade::Fades;
use crate::core::render::world::lights::{self, AMBIENT_NITS, Flicker};
use crate::core::render::world::terrain;
use crate::core::render::world::toon::{ToonMaterial, toon_of};
use crate::core::render::{GroundHeights, WorldCamera, ground_height, world_point};
use crate::core::terrain::MATERIALS;
use crate::core::units::Meters;
use crate::core::vfx::render::SiteExtent;
use crate::systems::player::session::MyCharacter;
use crate::systems::screen::ClientScreen;

pub struct AreaRenderPlugin;

impl Plugin for AreaRenderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ShownArea>()
            .add_systems(
                Update,
                (show_area, place_objects)
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
    let ambient = AmbientLight {
        color: area.lighting.ambient.color(),
        brightness: AMBIENT_NITS,
        ..default()
    };
    if let Ok(camera) = world
        .query_filtered::<Entity, With<WorldCamera>>()
        .single(world)
    {
        world.entity_mut(camera).insert(ambient);
    }
    spawn_terrain(world, &content, &area);
    for placed in &area.props {
        spawn_prop(world, &content, placed);
    }
    if let Some(sun) = &area.lighting.sun {
        world.spawn((
            lights::sun(sun.color.color(), sun.elevation, sun.azimuth),
            Scenery,
        ));
    }
    let mut rng = Rng::from_entropy();
    for light in &area.lights {
        spawn_light(world, light, &mut rng);
    }
    world.resource_mut::<ShownArea>().0 = Some(area_id);
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
    let material = world
        .resource_mut::<Assets<ToonMaterial>>()
        .add(toon_of(StandardMaterial::default()));
    world.spawn((
        Mesh3d(mesh),
        MeshMaterial3d(material),
        Transform::default(),
        Scenery,
    ));
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
