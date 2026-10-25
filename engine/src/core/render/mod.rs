pub mod camera;
mod icons;
pub mod pick;
pub mod present;
pub mod transition;
pub mod world;

pub use camera::{
    CameraPreferences, CameraRig, GroundHeights, PITCH, TILT_STEP, TURN_STEP, View, WorldCamera,
    Zoom, cursor_ground, cursor_ray, ground_height, ground_pos, view_of, window_point, world_point,
};
pub use pick::{Pointed, capsule_hit, pointed, sphere_hit};

use std::collections::HashMap;

use bevy::prelude::*;
use bevy::sprite_render::Material2dPlugin;

use crate::core::interpolate::RenderValueSystems;
use crate::core::time::Seconds;

pub struct WorldRenderPlugin;

impl Plugin for WorldRenderPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "present.wesl");
        app.add_plugins((
            Material2dPlugin::<present::Present>::default(),
            world::WorldScenePlugin,
        ))
        .init_resource::<Animator>()
        .init_resource::<CameraRig>()
        .init_resource::<CameraPreferences>()
        .init_resource::<GroundHeights>()
        .init_resource::<present::ScreenGrade>()
        .init_resource::<present::AreaGrade>()
        .init_resource::<present::PresentScale>()
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
        .add_systems(
            Update,
            (
                (present::match_display, present::fit).chain(),
                icons::keep_icons_resident,
                camera::ease,
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

#[derive(Resource, Default)]
pub struct Animator {
    anchors: HashMap<Entity, Anchor>,
}

#[derive(Clone, Copy)]
struct Anchor {
    state: u64,
    start: Seconds,
    played: Seconds,
}

/// Where an entity's animation stood at the frame before, in the state it is in now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Previous {
    Unseen,
    Restarted,
    Continued(Seconds),
}

pub struct Playhead {
    pub elapsed: Seconds,
    pub previous: Previous,
}

impl Animator {
    pub fn advance(&mut self, entity: Entity, state: u64, time: Seconds) -> Playhead {
        let anchor = self.anchors.get(&entity).copied();
        let (previous, start) = match anchor {
            Some(anchor) if anchor.state == state => {
                (Previous::Continued(anchor.played), anchor.start)
            }
            Some(_) => (Previous::Restarted, time),
            None => (Previous::Unseen, time),
        };
        let elapsed = time - start;
        self.anchors.insert(
            entity,
            Anchor {
                state,
                start,
                played: elapsed,
            },
        );
        Playhead { elapsed, previous }
    }

    pub fn retain(&mut self, mut alive: impl FnMut(Entity) -> bool) {
        self.anchors.retain(|entity, _| alive(*entity));
    }
}
