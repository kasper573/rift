use crate::systems::player::session;
use bevy::prelude::*;
use bevy::window::CursorIcon;

use crate::systems::input::gestures::{Gesture, content_cursor};
use crate::systems::interface::CursorShape;

pub struct LockedGesture;

impl Gesture for LockedGesture {
    fn priority(&self) -> i32 {
        1
    }

    fn claims(&self, world: &mut World) -> bool {
        session::is_locked(world)
    }

    fn drive(&self, _world: &mut World, _start: bool) {}

    fn cursor(&self, world: &mut World) -> Option<CursorIcon> {
        Some(content_cursor(world, CursorShape::Default))
    }
}
