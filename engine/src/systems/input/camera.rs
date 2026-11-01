//! Steering the camera: keys turn it a step, tilt it a step and bring it home, the wheel zooms
//! it, and dragging with the middle button turns it freely.

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use bevy_egui::input::egui_wants_any_keyboard_input;

use super::map::{self, InputAction};
use crate::core::geometry::Degrees;
use crate::core::gui;
use crate::core::render::{CameraFraming, CameraRig};
use crate::systems::screen::ClientScreen;

/// How far the camera turns for a pixel of middle-button drag.
const DRAG_TURN: Degrees = Degrees(0.4);
/// How many scrolled lines take the zoom one step.
const LINES_PER_STEP: f32 = 1.0;
const PIXELS_PER_STEP: f32 = 60.0;

pub fn plugin(app: &mut App) {
    app.init_resource::<Scrolled>().add_systems(
        Update,
        (
            steer_by_keys.run_if(not(egui_wants_any_keyboard_input)),
            steer_by_pointer,
        )
            .run_if(in_state(ClientScreen::Area)),
    );
}

/// Scrolling gathered toward the next zoom step.
#[derive(Resource, Default)]
struct Scrolled(f32);

fn steer_by_keys(world: &mut World) {
    let left = map::just_pressed(world, InputAction::TurnLeft);
    let right = map::just_pressed(world, InputAction::TurnRight);
    let up = map::just_pressed(world, InputAction::TiltUp);
    let down = map::just_pressed(world, InputAction::TiltDown);
    let home = map::just_pressed(world, InputAction::CameraHome);
    let Some(mut rig) = world.get_resource_mut::<CameraRig>() else {
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
}

fn steer_by_pointer(world: &mut World) {
    if gui::pointer_claimed(world) {
        return;
    }
    let scroll = world.resource::<AccumulatedMouseScroll>();
    let steps = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y / LINES_PER_STEP,
        MouseScrollUnit::Pixel => scroll.delta.y / PIXELS_PER_STEP,
    };
    let dragged = world
        .resource::<ButtonInput<MouseButton>>()
        .pressed(MouseButton::Middle)
        .then(|| world.resource::<AccumulatedMouseMotion>().delta.x);
    let mut gathered = world.resource_mut::<Scrolled>();
    gathered.0 += steps;
    let zoom = gathered.0.trunc();
    gathered.0 -= zoom;
    let Some(mut rig) = world.get_resource_mut::<CameraRig>() else {
        return;
    };
    for _ in 0..zoom.abs() as u32 {
        if zoom > 0.0 {
            rig.zoom_in();
        } else {
            rig.zoom_out();
        }
    }
    if let Some(pixels) = dragged
        && pixels != 0.0
    {
        rig.turn_now(Degrees(-pixels * DRAG_TURN.0));
    }
}
