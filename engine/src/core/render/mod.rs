pub mod camera;
mod icons;
pub mod pick;
pub mod present;
pub mod sample;
pub mod transition;
pub mod video;
pub mod world;

pub use camera::{
    CameraFocus, CameraFraming, CameraPreferences, CameraRig, CameraRigSystems, GroundHeights,
    Orbit, PictureGrade, Shot, View, WorldCamera, Zoom, Zooms, cursor_ground, cursor_ray,
    facing_rotation, ground_height, ground_pos, view_of, window_point, window_size, world_point,
};
pub use pick::{Pointed, capsule_hit, pointed};

use std::collections::HashMap;

use bevy::prelude::*;
use bevy::render::view::NoIndirectDrawing;
use bevy::sprite_render::Material2dPlugin;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::interpolate::RenderValueSystems;
use crate::core::math::Rng;
use crate::core::units::Seconds;
pub use world::clouds::CloudLook;
pub use world::gpu::clearing::ClearingLook;
pub use world::lights::LightLook;
pub use world::material::SurfaceFinish;
pub use world::sky::SkyLook;

pub struct WorldRenderPlugin;

impl Plugin for WorldRenderPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "present.wesl");
        app.add_systems(PostUpdate, draw_directly);
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
        .configure_sets(
            Update,
            CameraRigSystems.before(transition::WorldViewSystems),
        )
        .add_systems(Startup, present::setup)
        .add_systems(First, take_look.run_if(resource_exists_and_changed::<Look>))
        .add_systems(
            Update,
            (
                present::fit,
                icons::keep_icons_resident,
                camera::ease.in_set(CameraRigSystems),
                camera::stand_over_ground
                    .after(CameraRigSystems)
                    .before(transition::WorldViewSystems),
            ),
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
#[derive(Resource, JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Look {
    pub camera: CameraFraming,
    pub picture: PictureGrade,
    pub sky: SkyLook,
    pub clouds: CloudLook,
    pub light: LightLook,
    pub materials: SurfaceFinish,
    pub clearing: ClearingLook,
    /// How long a thing takes to fade in or out as it comes into the world or leaves it within
    /// a player's sight; at once without it.
    #[serde(default, skip_serializing_if = "Seconds::is_zero")]
    pub presence_fades: Seconds,
}

/// Frames the camera and finishes the world's picture as the game's look says, once it is known
/// and whenever it changes.
fn take_look(world: &mut World) {
    let look = *world.resource::<Look>();
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

impl FromWorld for Animator {
    fn from_world(world: &mut World) -> Animator {
        Animator {
            anchors: HashMap::new(),
            rng: Rng::fork_of(world),
        }
    }
}

#[derive(Clone, Copy)]
struct Anchor {
    state: u64,
    turn: ClipTurn,
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

/// One turn of a state's clips: the clip picked, and how often it plays through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClipTurn {
    pub pick: usize,
    pub plays: u32,
}

/// How a state that loops plays on: how long the clip picked plays through once, whether the
/// state begins anywhere in it rather than at its start, and, where the state's clips take turns,
/// the turn that follows the clip picked before, or the first for none.
pub struct Loops<'a> {
    pub length: &'a dyn Fn(usize) -> Seconds,
    pub anywhere: &'a dyn Fn(usize) -> bool,
    pub turns: Option<&'a mut NextTurn<'a>>,
}

/// Picks the turn that follows the clip picked before, or the first for none.
pub type NextTurn<'a> = dyn FnMut(Option<usize>, &mut Rng) -> ClipTurn + 'a;

impl Animator {
    /// Moves the entity's animation on to `time` in `state`, as `loops` has it play on when it
    /// loops: a state that begins anywhere begins a random way into its first turn, so no two
    /// entities move in step.
    pub fn advance(
        &mut self,
        entity: Entity,
        state: u64,
        time: Seconds,
        loops: Option<Loops>,
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
                    turn: ClipTurn { pick: 0, plays: 1 },
                    since: time,
                    played: Seconds(0.0),
                },
            ),
        };
        if let Some(Loops {
            length,
            anywhere,
            mut turns,
        }) = loops
        {
            let lasts = |turn: ClipTurn| length(turn.pick) * turn.plays as f32;
            if previous != Previous::Continued(anchor.played) {
                if let Some(next) = turns.as_mut() {
                    anchor.turn = next(None, &mut self.rng);
                }
                if anywhere(anchor.turn.pick) {
                    anchor.since = time - lasts(anchor.turn) * self.rng.rand_float();
                }
            }
            if let Some(next) = turns {
                loop {
                    let lasts = lasts(anchor.turn);
                    if lasts.0 <= 0.0 || time - anchor.since < lasts {
                        break;
                    }
                    anchor.since += lasts;
                    anchor.turn = next(Some(anchor.turn.pick), &mut self.rng);
                    previous = Previous::Restarted;
                }
            }
        }
        let elapsed = time - anchor.since;
        anchor.played = elapsed;
        self.anchors.insert(entity, anchor);
        Playhead {
            elapsed,
            previous,
            turn: anchor.turn.pick,
        }
    }

    pub fn retain(&mut self, mut alive: impl FnMut(Entity) -> bool) {
        self.anchors.retain(|entity, _| alive(*entity));
    }
}

/// Every camera draws each of bevy's meshes directly. Everything that stands in numbers (the
/// ground, what grows on it, the props) is culled and drawn by the engine's own kernels, which
/// leaves bevy a handful of meshes: bodies, the sky, the rain, the marks. Culling those on the
/// GPU into indirect draws costs every view the same passes and bindings however few they are.
fn draw_directly(mut commands: Commands, cameras: Query<Entity, Added<Camera3d>>) {
    for camera in &cameras {
        commands.entity(camera).insert(NoIndirectDrawing);
    }
}
