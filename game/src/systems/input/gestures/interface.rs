use bevy::prelude::*;
use bevy::window::{CursorIcon, SystemCursorIcon};
use ui::{CursorStyle, InterfaceCursor};

use crate::systems::input::gestures::{Gesture, content_cursor};
use crate::systems::interface::CursorShape;

pub struct InterfaceGesture;

impl Gesture for InterfaceGesture {
    fn priority(&self) -> i32 {
        0
    }

    fn claims(&self, world: &mut World) -> bool {
        world.resource::<InterfaceCursor>().captured()
    }

    fn drive(&self, _world: &mut World, _start: bool) {}

    fn cursor(&self, world: &mut World) -> Option<CursorIcon> {
        let shape = match world.resource::<InterfaceCursor>().style()? {
            CursorStyle::Default => CursorShape::Default,
            CursorStyle::Pointer => CursorShape::Pointer,
            CursorStyle::Grab => CursorShape::Grab,
            CursorStyle::Grabbing => CursorShape::Grabbing,
            CursorStyle::Resize => CursorShape::Resize,
            CursorStyle::Text => return Some(CursorIcon::System(SystemCursorIcon::Text)),
        };
        Some(content_cursor(world, shape))
    }
}
