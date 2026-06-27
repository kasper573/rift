use bevy::prelude::*;
use bevy::window::CursorIcon;

use crate::systems::input::gestures::{Gesture, image_cursor};

/// The catch-all gesture: it claims any press the more specific gestures pass on, but does nothing with
/// it. It exists so the pointer always has a cursor (the idle one) and a press always has an owner.
pub struct DefaultGesture;

inventory::submit! {
    &DefaultGesture as &dyn Gesture
}

impl Gesture for DefaultGesture {
    fn priority(&self) -> i32 {
        4
    }

    fn claims(&self, _world: &mut World) -> bool {
        true
    }

    fn drive(&self, _world: &mut World, _start: bool) {}

    fn cursor(&self, world: &mut World) -> Option<CursorIcon> {
        let handle = world
            .resource::<AssetServer>()
            .load("icons/cursors/pointer003.png");
        Some(image_cursor(handle, (0, 0)))
    }
}
