use crate::systems::player::session;
use bevy::prelude::*;
use bevy::window::CursorIcon;

use crate::systems::input::gestures::{Gesture, image_cursor};

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
        let handle = world
            .resource::<AssetServer>()
            .load("icons/cursors/denied001.png");
        Some(image_cursor(handle, (32, 32)))
    }
}
