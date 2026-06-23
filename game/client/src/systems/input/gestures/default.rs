use bevy::prelude::*;
use bevy::window::CursorIcon;

use crate::systems::input::gestures::{Gesture, image_cursor};

/// The catch-all gesture: it claims any press the more specific gestures pass on, but does nothing with
/// it. It exists so the pointer always has a cursor (the idle one) and a press always has an owner.
pub struct DefaultGesture {
    image: Handle<Image>,
}

impl DefaultGesture {
    pub fn new(assets: &AssetServer) -> DefaultGesture {
        DefaultGesture {
            image: assets.load("icons/cursors/pointer003.png"),
        }
    }
}

impl Gesture for DefaultGesture {
    fn claims(&self, _world: &mut World) -> bool {
        true
    }

    fn drive(&mut self, _world: &mut World, _start: bool) {}

    fn cursor(&self, _world: &mut World) -> Option<CursorIcon> {
        Some(image_cursor(self.image.clone(), (0, 0)))
    }
}
