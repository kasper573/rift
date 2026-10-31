//! The area on view, drawn: its scenery stood when the player's character arrives, lit again as
//! the day moves on, and torn down when the view leaves it.

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;

use super::scenery::{self, Scenery, SceneryPlugin};
use super::{AreaId, AreaObject, AreaTag};
use crate::core::content::Content;
use crate::core::math::{Pos, Rect};
use crate::core::render::camera::Zoom;
use crate::core::render::transition::WorldViewSystems;
use crate::core::render::world::sky::Daylight;
use crate::core::render::world::terrain;
use crate::core::render::{GroundHeights, WorldCamera, ground_height, world_point};
use crate::core::units::{MeterSize, Meters};
use crate::core::vfx::render::SiteExtent;
use crate::systems::movement::RenderPosition;
use crate::systems::player::session::MyCharacter;
use crate::systems::screen::ClientScreen;

pub struct AreaRenderPlugin;

impl Plugin for AreaRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(SceneryPlugin)
            .init_resource::<ShownArea>()
            .init_resource::<Daylight>()
            .add_systems(
                Update,
                (
                    super::weather::tell_time.run_if(resource_exists::<Content>),
                    show_area,
                    follow_view,
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

/// The area the view is set up for, its scenery and the hour its sun was last turned to.
#[derive(Resource, Default)]
struct ShownArea(Option<Standing>);

struct Standing {
    area: AreaId,
    scenery: Scenery,
    shone: Option<Daylight>,
}

/// The area whose scenery stands in the view.
pub fn shown(world: &World) -> Option<AreaId> {
    world
        .resource::<ShownArea>()
        .0
        .as_ref()
        .map(|standing| standing.area)
}

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
    let image = world
        .resource_mut::<Assets<Image>>()
        .add(terrain::height_image(&area.heights));
    world.insert_resource(GroundHeights {
        heights: area.heights.clone(),
        bounds: area.size.bounds(),
        image,
    });
    world.insert_resource(scenery::grade(&area.lighting));
    world.insert_resource(scenery::climate(&area));
    let stood = scenery::spawn(world, &content, &area, RenderLayers::default());
    world.resource_mut::<ShownArea>().0 = Some(Standing {
        area: area_id,
        scenery: stood,
        shone: None,
    });
    light_area(world, &area);
}

/// Tells the shown scenery where the player's character stands, so what stands around them
/// follows them: as far as the camera sees from its widest, and the fades past that.
fn follow_view(world: &mut World) {
    let Some(stood) = world
        .resource::<ShownArea>()
        .0
        .as_ref()
        .map(|standing| standing.scenery)
    else {
        return;
    };
    let Some(at) = world
        .resource::<MyCharacter>()
        .0
        .and_then(|seen| world.get::<RenderPosition>(seen))
        .map(|position| position.0)
    else {
        return;
    };
    scenery::view_from(world, stood, at, view_reach());
}

/// How far around the character the covers, props and lights stand: the ground the camera takes
/// in from its widest, and as far again for what the frame's edge fades.
fn view_reach() -> Meters {
    Meters(Zoom::Wide.distance().0 * 4.0)
}

/// Turns the shown area's sun with the hour as it moves, and its clouds with the wind.
fn follow_daylight(world: &mut World) {
    let Some(area_id) = shown(world) else {
        return;
    };
    let content = world.resource::<Content>().clone();
    let area = super::shared(&content, area_id).clone();
    light_area(world, &area);
}

fn light_area(world: &mut World, area: &super::Area) {
    let daylight = *world.resource::<Daylight>();
    let Some(stood) = world
        .resource::<ShownArea>()
        .0
        .as_ref()
        .map(|standing| standing.scenery)
    else {
        return;
    };
    let Ok(camera) = world
        .query_filtered::<Entity, With<WorldCamera>>()
        .single(world)
    else {
        return;
    };
    scenery::light(world, area, daylight, stood, camera);
    if let Some(standing) = world.resource_mut::<ShownArea>().0.as_mut() {
        standing.shone = Some(daylight);
    }
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
}

fn tear_down(world: &mut World) {
    if let Some(standing) = world.resource_mut::<ShownArea>().0.take() {
        scenery::despawn(world, standing.scenery);
    }
}
