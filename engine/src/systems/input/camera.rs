//! Steering the camera: its actions turn it a step, tilt it a step, zoom it a step and bring it
//! home, and moving the pointer while turning or tilting it is held turns or tilts it freely, the
//! camera trailing the pointer as it eases. A camera put on a shot is not steered.

use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;

use super::map::{self, InputAction};
use crate::core::gui;
use crate::core::render::{CameraFraming, CameraRig, CameraRigSystems};
use crate::core::units::Degrees;
use crate::systems::screen::ClientScreen;

/// How far the camera turns or tilts for a pixel the pointer moves while it is held to.
pub const DRAG_TURN: Degrees = Degrees(0.4);

pub fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        (steer_by_keys, steer_by_pointer)
            .in_set(CameraRigSystems)
            .run_if(in_state(ClientScreen::Area)),
    );
}

fn steer_by_keys(world: &mut World) {
    let left = map::just_pressed(world, InputAction::TurnLeft);
    let right = map::just_pressed(world, InputAction::TurnRight);
    let up = map::just_pressed(world, InputAction::TiltUp);
    let down = map::just_pressed(world, InputAction::TiltDown);
    let home = map::just_pressed(world, InputAction::CameraHome);
    let closer = u32::from(map::just_pressed(world, InputAction::ZoomIn));
    let wider = u32::from(map::just_pressed(world, InputAction::ZoomOut));
    let Some(mut rig) = world
        .get_resource_mut::<CameraRig>()
        .filter(|rig| !rig.staged())
    else {
        return;
    };
    let CameraFraming {
        turn_step,
        tilt_step,
        ..
    } = *rig.framing();
    if left {
        rig.turn(turn_step);
    }
    if right {
        rig.turn(Degrees(-turn_step.0));
    }
    if up {
        rig.tilt(Degrees(-tilt_step.0));
    }
    if down {
        rig.tilt(tilt_step);
    }
    if home {
        rig.home();
    }
    zoom(&mut rig, closer, wider);
}

fn steer_by_pointer(world: &mut World) {
    if gui::pointer_claimed(world) {
        return;
    }
    let closer = map::wheeled(world, InputAction::ZoomIn);
    let wider = map::wheeled(world, InputAction::ZoomOut);
    let moved = world.resource::<AccumulatedMouseMotion>().delta;
    let turned = map::pressed(world, InputAction::TurnCamera).then_some(moved.x);
    let tilted = map::pressed(world, InputAction::TiltCamera).then_some(moved.y);
    let Some(mut rig) = world
        .get_resource_mut::<CameraRig>()
        .filter(|rig| !rig.staged())
    else {
        return;
    };
    zoom(&mut rig, closer, wider);
    if let Some(pixels) = turned
        && pixels != 0.0
    {
        rig.turn(Degrees(-pixels * DRAG_TURN.0));
    }
    if let Some(pixels) = tilted
        && pixels != 0.0
    {
        rig.tilt(Degrees(pixels * DRAG_TURN.0));
    }
}

fn zoom(rig: &mut Mut<CameraRig>, closer: u32, wider: u32) {
    for _ in 0..closer {
        rig.zoom_in();
    }
    for _ in 0..wider {
        rig.zoom_out();
    }
}
