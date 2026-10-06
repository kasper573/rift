use bevy::prelude::*;
use bevy::window::{CursorIcon, SystemCursorIcon};
use ui::{CursorStyle, InterfaceCursor};

use crate::systems::input::gestures::{Gesture, default_cursor, image_cursor};

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
        let image = |world: &mut World, path: &'static str, hotspot: (u16, u16)| {
            image_cursor(world.resource::<AssetServer>().load(path), hotspot)
        };
        Some(match world.resource::<InterfaceCursor>().style()? {
            CursorStyle::Default => default_cursor(world),
            CursorStyle::Pointer => image(world, "icons/cursors/hand002.png", (3, 3)),
            CursorStyle::Grab => image(world, "icons/cursors/hand001.png", (28, 30)),
            CursorStyle::Grabbing => image(world, "icons/cursors/hand003.png", (32, 30)),
            CursorStyle::Resize => image(world, "icons/cursors/move006.png", (32, 31)),
            CursorStyle::Text => CursorIcon::System(SystemCursorIcon::Text),
        })
    }
}
