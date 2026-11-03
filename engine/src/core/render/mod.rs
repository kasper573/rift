pub mod camera;
mod icons;
pub mod pick;
pub mod present;
pub mod transition;
pub mod video;
pub mod world;

pub use camera::{
    CameraFraming, CameraPreferences, CameraRig, GroundHeights, PictureGrade, View, WorldCamera,
    Zoom, Zooms, cursor_ground, cursor_ray, facing_rotation, ground_height, ground_pos, view_of,
    window_point, world_point,
};
pub use pick::{Pointed, capsule_hit, pointed};

use std::collections::HashMap;

use bevy::prelude::*;
use bevy::render::batching::gpu_preprocessing::{GpuPreprocessingMode, GpuPreprocessingSupport};
use bevy::render::{RenderApp, RenderStartup, init_gpu_resource};
use bevy::sprite_render::Material2dPlugin;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::interpolate::RenderValueSystems;
use crate::core::math::Rng;
use crate::core::units::Seconds;
pub use world::clouds::CloudLook;
pub use world::fade::ClearingLook;
pub use world::lights::LightLook;
pub use world::material::SurfaceFinish;
pub use world::sky::SkyLook;

pub struct WorldRenderPlugin;

impl Plugin for WorldRenderPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "present.wesl");
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app.add_systems(
                RenderStartup,
                draw_directly.after(init_gpu_resource::<GpuPreprocessingSupport>),
            );
        }
        app.add_plugins((
            Material2dPlugin::<present::Present>::default(),
            world::WorldScenePlugin,
        ))
        .init_resource::<Animator>()
        .init_resource::<CameraPreferences>()
        .init_resource::<present::ScreenGrade>()
        .init_resource::<present::AreaGrade>()
        .init_resource::<transition::ActiveTransition>()
        .init_resource::<transition::ScreenTransitionPreference>()
        .init_resource::<icons::ResidentIcons>()
        .configure_sets(
            PreUpdate,
            transition::WorldViewSystems.run_if(not(transition::covering)),
        )
        .configure_sets(
            Update,
            transition::WorldViewSystems
                .run_if(not(transition::covering))
                .after(RenderValueSystems),
        )
        .add_systems(Startup, present::setup)
        .add_systems(First, take_look.run_if(resource_exists_and_changed::<Look>))
        .add_systems(
            Update,
            (present::fit, icons::keep_icons_resident, camera::ease),
        )
        .add_systems(
            PostUpdate,
            (
                camera::place.before(bevy::transform::TransformSystems::Propagate),
                present::apply_present.after(bevy::transform::TransformSystems::Propagate),
            ),
        );
    }
}

/// How a game looks: how its camera frames the world, how the picture is finished, the air and
/// clouds over the world and the light they let through, how its surfaces take the light, and
/// how what hides the player clears away.
#[derive(Resource, JsonSchema, Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Look {
    pub camera: CameraFraming,
    pub picture: PictureGrade,
    pub sky: SkyLook,
    pub clouds: CloudLook,
    pub light: LightLook,
    pub materials: SurfaceFinish,
    pub clearing: ClearingLook,
}

/// Frames the camera and finishes the world's picture as the game's look says, once it is known
/// and whenever it changes.
fn take_look(world: &mut World) {
    let look = world.resource::<Look>().clone();
    match world.get_resource_mut::<CameraRig>() {
        Some(mut rig) => rig.reframe(look.camera),
        None => world.insert_resource(CameraRig::new(look.camera)),
    }
    present::take_look(world, &look);
}

/// Where each entity's animation stands: the state it is in, and the turn of the state's clips it
/// is on, each turn picked at random by its own hand, since which clip shows is for the eye alone.
#[derive(Resource)]
pub struct Animator {
    anchors: HashMap<Entity, Anchor>,
    rng: Rng,
}

impl Default for Animator {
    fn default() -> Animator {
        Animator {
            anchors: HashMap::new(),
            rng: Rng::from_entropy(),
        }
    }
}

#[derive(Clone, Copy)]
struct Anchor {
    state: u64,
    turn: usize,
    since: Seconds,
    played: Seconds,
}

/// Where an entity's animation stood at the frame before, in the state and turn it is in now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Previous {
    Unseen,
    Restarted,
    Continued(Seconds),
}

/// How far into its turn an entity's animation is, and which turn it is on.
pub struct Playhead {
    pub elapsed: Seconds,
    pub previous: Previous,
    pub turn: usize,
}

/// How a state's clips take turns as it plays on: how long each turn plays, and the turn that
/// follows another, or the first for none.
pub struct Turns<'a> {
    pub length: &'a dyn Fn(usize) -> Seconds,
    pub next: &'a mut dyn FnMut(Option<usize>, &mut Rng) -> usize,
}

impl Animator {
    /// Moves the entity's animation on to `time` in `state`, taking `turns` when its clips take
    /// them, and keeping to one clip when they don't.
    pub fn advance(
        &mut self,
        entity: Entity,
        state: u64,
        time: Seconds,
        turns: Option<Turns>,
    ) -> Playhead {
        let (mut previous, mut anchor) = match self.anchors.get(&entity).copied() {
            Some(anchor) if anchor.state == state => (Previous::Continued(anchor.played), anchor),
            seen => (
                match seen {
                    Some(_) => Previous::Restarted,
                    None => Previous::Unseen,
                },
                Anchor {
                    state,
                    turn: 0,
                    since: time,
                    played: Seconds(0.0),
                },
            ),
        };
        if let Some(Turns { length, next }) = turns {
            if previous != Previous::Continued(anchor.played) {
                anchor.turn = next(None, &mut self.rng);
            }
            loop {
                let played_once = length(anchor.turn);
                if played_once.0 <= 0.0 || time - anchor.since < played_once {
                    break;
                }
                anchor.since += played_once;
                anchor.turn = next(Some(anchor.turn), &mut self.rng);
                previous = Previous::Restarted;
            }
        }
        let elapsed = time - anchor.since;
        anchor.played = elapsed;
        self.anchors.insert(entity, anchor);
        Playhead {
            elapsed,
            previous,
            turn: anchor.turn,
        }
    }

    pub fn retain(&mut self, mut alive: impl FnMut(Entity) -> bool) {
        self.anchors.retain(|entity, _| alive(*entity));
    }
}

/// Every view draws directly, with no GPU culling: bevy's GPU-driven indirect drawing costs more
/// CPU a frame than it saves on a scene of a few hundred meshes (half the frame rate on the client
/// bench). Capped for the whole renderer rather than per camera, so the sun's shadow maps draw the
/// same way as the cameras.
fn draw_directly(mut support: ResMut<GpuPreprocessingSupport>) {
    support.max_supported_mode = support.min(GpuPreprocessingMode::PreprocessingOnly);
}
