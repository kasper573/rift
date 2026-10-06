use bevy::prelude::*;
use bevy::window::CursorIcon;

use crate::systems::input::gestures::{Gesture, default_cursor};

pub struct DefaultGesture;

impl Gesture for DefaultGesture {
    fn priority(&self) -> i32 {
        6
    }

    fn claims(&self, _world: &mut World) -> bool {
        true
    }

    fn drive(&self, _world: &mut World, _start: bool) {}

    fn cursor(&self, world: &mut World) -> Option<CursorIcon> {
        Some(default_cursor(world))
    }
}
