//! Where this frame's props clear away for hiding the camera's subject, as the culling kernel and
//! the props' pixels read it: whom the camera follows, how far a fade moves this frame, and the
//! circle round the subject on the picture that a prop in the way clears.

use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy::render::extract_resource::ExtractResource;
use bevy::window::PrimaryWindow;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::render::Look;
use crate::core::render::camera::{CameraRig, WorldCamera};
use crate::core::units::{Meters, Seconds};

/// The clearing of the frame: none while no camera follows anyone on screen.
#[derive(Resource, ExtractResource, Clone, Copy, Debug, Default, PartialEq)]
#[extract_app(RenderApp)]
pub struct Clearing(pub Option<ClearingAim>);

/// How what hides the camera's subject clears away.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ClearingLook {
    /// How long a fade takes, out or back.
    pub fades: Seconds,
    /// How far round the subject a fading thing is cleared, on the ground it stands on.
    pub radius: Meters,
}

/// What the frame clears, as `camera` sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClearingAim {
    pub camera: Entity,
    pub target: Vec3,
    /// How far a fade moves this frame, as a share of the way from whole to cleared.
    pub step: f32,
    /// Where the subject shows on the picture, and how many pixels round it a prop in the way
    /// clears.
    pub focus: Vec2,
    pub focus_radius: f32,
}

/// Aims the frame's clearing from the camera's rig.
pub(crate) fn aim_clearing(
    mut clearing: ResMut<Clearing>,
    time: Res<Time>,
    rig: Option<Res<CameraRig>>,
    look: Option<Res<Look>>,
    window: Query<&Window, With<PrimaryWindow>>,
    camera: Query<Entity, With<WorldCamera>>,
) {
    let aimed = (|| {
        let (rig, look) = (rig?, look?);
        let window = window.single().ok()?;
        let camera = camera.single().ok()?;
        let target = rig.target();
        let view = rig.view(Vec2::new(
            window.resolution.width(),
            window.resolution.height(),
        ));
        let focus = view.project(target)?;
        let rim = view.project(target + rig.transform().right() * look.clearing.radius.0)?;
        Some(ClearingAim {
            camera,
            target,
            step: time.delta_secs() / look.clearing.fades.0.max(f32::EPSILON),
            focus,
            focus_radius: focus.distance(rim),
        })
    })();
    clearing.set_if_neq(Clearing(aimed));
}
