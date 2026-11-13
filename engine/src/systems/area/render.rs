//! The area on view, drawn: its scenery stood when the player's character arrives, lit again as
//! the day moves on, and torn down when the view leaves it.

use std::sync::Arc;

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;

use super::scenery::{self, Eyes, Scenery, SceneryPlugin, Sight};
use super::{AreaId, AreaObject, AreaTag, PropDef};
use crate::core::assets;
use crate::core::content::Content;
use crate::core::content::media::GltfInfos;
use crate::core::render::transition::WorldViewSystems;
use crate::core::render::video::VideoPreferences;
use crate::core::render::world::sky::Daylight;
use crate::core::render::{CameraFraming, CameraRig, camera};
use crate::core::render::{GroundHeights, WorldCamera, ground_height, world_point};
use crate::core::units::{MeterSize, Meters};
use crate::core::vfx::cover::ShownWeather;
use crate::core::vfx::render::SiteExtent;
use crate::core::vfx::standing::StandingVfx;
use crate::systems::area::rain::RainHeard;
use crate::systems::movement::RenderPosition;
use crate::systems::player::session::MyCharacter;
use crate::systems::screen::ClientScreen;
use crate::systems::script::sky::{self, Sky};

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
                    play_prop_effects,
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
    world.insert_resource(GroundHeights {
        heights: area.heights.clone(),
        bounds: area.size.bounds(),
    });
    world.insert_resource(scenery::grade(&area.lighting));
    let weather = super::weather::weather_now(world, area_id, &area.climate);
    world.insert_resource(scenery::shown_weather(&weather, 0.0));
    let stood = scenery::spawn(world, &content, &area, RenderLayers::default());
    world.resource_mut::<ShownArea>().0 = Some(Standing {
        area: area_id,
        scenery: stood,
        shone: None,
    });
    light_area(world, area_id, &area, &weather);
}

/// Tells the shown scenery what the camera looks at, from wherever its framing lets it stand,
/// once the player's character is there for it to follow: so what stands around them follows
/// them, as far as the camera sees from its widest, and the fades past that.
fn follow_view(world: &mut World) {
    let Some(stood) = world
        .resource::<ShownArea>()
        .0
        .as_ref()
        .map(|standing| standing.scenery)
    else {
        return;
    };
    let followed = world
        .resource::<MyCharacter>()
        .0
        .is_some_and(|seen| world.get::<RenderPosition>(seen).is_some());
    let Some(lens) = camera::view_of(world)
        .filter(|_| followed)
        .map(|view| view.lens())
    else {
        return;
    };
    let Some(rig) = world.get_resource::<CameraRig>() else {
        return;
    };
    let target = rig.target();
    let at = camera::ground_pos(target);
    let lift = Meters(target.y - ground_height(world, at).0);
    let video = world.resource::<VideoPreferences>();
    let sight = Sight {
        at,
        reach: view_reach(rig.framing(), video),
        horizon: video.view_distance.horizon(),
        lens,
        eyes: Eyes::orbiting(rig.framing(), lift),
    };
    scenery::view_from(world, stood, sight);
}

/// How far around the character the covers, props and lights stand: the ground the camera takes
/// in from its widest, and as far again for what the frame's edge fades, as much of it as the
/// player's view distance takes in.
pub fn view_reach(framing: &CameraFraming, video: &VideoPreferences) -> Meters {
    Meters(framing.zooms.wide.0 * 4.0 * video.view_distance.share())
}

/// Turns the shown area's sun with the hour as it moves, its clouds with the wind, and lets its
/// rain fall.
fn follow_daylight(world: &mut World) {
    let Some(area_id) = shown(world) else {
        return;
    };
    let content = world.resource::<Content>().clone();
    let area = super::shared(&content, area_id).clone();
    let weather = super::weather::weather_now(world, area_id, &area.climate);
    let hour = match *world.resource::<Daylight>() {
        Daylight::Hour(hour) => Some(hour),
        Daylight::Fixed => None,
    };
    let rain = world
        .get_resource::<RainHeard>()
        .map_or(0.0, |heard| heard.0);
    sky::turn(
        world,
        Sky {
            hour,
            weather,
            rain,
        },
    );
    let flash = light_area(world, area_id, &area, &weather);
    let shown = scenery::shown_weather(&weather, flash);
    match world.get_resource_mut::<ShownWeather>() {
        Some(mut was) => {
            was.set_if_neq(shown);
        }
        None => world.insert_resource(shown),
    }
}

/// Lights the shown scenery of `area` under `weather` and lets its rain fall, returning how
/// brightly lightning lights it this moment.
fn light_area(
    world: &mut World,
    area_id: AreaId,
    area: &Arc<super::Area>,
    weather: &super::Weather,
) -> f32 {
    let daylight = *world.resource::<Daylight>();
    let Some(stood) = world
        .resource::<ShownArea>()
        .0
        .as_ref()
        .map(|standing| standing.scenery)
    else {
        return 0.0;
    };
    let Ok(camera) = world
        .query_filtered::<Entity, With<WorldCamera>>()
        .single(world)
    else {
        return 0.0;
    };
    scenery::light(world, area, weather, daylight, stood, camera);
    if let Some(mut shown) = world.get_resource_mut::<ShownArea>()
        && shown
            .0
            .as_ref()
            .is_some_and(|standing| standing.shone != Some(daylight))
        && let Some(standing) = shown.0.as_mut()
    {
        standing.shone = Some(daylight);
    }
    super::rain::fall(world, area_id, area, weather, stood.rain, camera)
}

/// Gives every object of the shown world a place on the ground, the extent of its shape and the
/// prop it looks like, so it stands and what plays on it finds it where the area puts it.
fn place_objects(world: &mut World) {
    let objects: Vec<(Entity, AreaObject)> = world
        .query_filtered::<(Entity, &AreaObject), Without<SiteExtent>>()
        .iter(world)
        .map(|(entity, object)| (entity, object.clone()))
        .collect();
    for (entity, object) in objects {
        let (center, bounds) = (object.shape.center(), object.shape.bounds());
        let ground = ground_height(world, center);
        let model = object.look.and_then(|look| {
            let content = world.get_resource::<Content>()?;
            let prop = content.by_path::<PropDef>(&look)?.get(content);
            Some(assets::model_scene(
                world.resource::<AssetServer>(),
                prop.model.path(),
            ))
        });
        let mut placed = world.entity_mut(entity);
        placed.insert((
            Transform::from_translation(world_point(center, ground))
                .with_rotation(Quat::from_rotation_y(-object.yaw.radians().0)),
            Visibility::default(),
            SiteExtent {
                footprint: Vec2::new(bounds.size.width.max(1.0), bounds.size.height.max(1.0)),
                height: 1.0,
            },
        ));
        if let Some(model) = model {
            placed.insert(WorldAssetRoot(model));
        }
    }
}

/// A model whose prop's own vfx have been looked for.
#[derive(Component)]
struct EffectsSought;

/// Has every model newly shown play the vfx of the prop it is the model of, wherever it is
/// shown: as an object's look, lying on the ground or held in a hand. A model no one gave an
/// extent takes that of its mesh.
fn play_prop_effects(world: &mut World) {
    let content = world.resource::<Content>().clone();
    let fresh: Vec<(Entity, String)> = world
        .query_filtered::<(Entity, &WorldAssetRoot), Without<EffectsSought>>()
        .iter(world)
        .filter_map(|(entity, root)| {
            let model = root.0.path()?.path().to_str()?.to_owned();
            Some((entity, model))
        })
        .collect();
    for (entity, model) in fresh {
        let mut shown = world.entity_mut(entity);
        shown.insert(EffectsSought);
        let prop = std::path::Path::new(&model)
            .parent()
            .and_then(|bundle| content.by_path::<PropDef>(bundle.to_str()?))
            .map(|prop| prop.get(&content))
            .filter(|prop| !prop.effects.is_empty());
        let Some(prop) = prop else {
            continue;
        };
        if !shown.contains::<SiteExtent>()
            && let Some(info) = content.built::<GltfInfos>().0.get(&model)
        {
            shown.insert(scenery::extent_of(info, 1.0));
        }
        shown.insert(StandingVfx(prop.effects.clone()));
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
