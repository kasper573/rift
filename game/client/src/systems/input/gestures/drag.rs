use bevy::picking::hover::HoverMap;
use bevy::prelude::*;
use bevy::window::{CursorIcon, SystemCursorIcon};
use ui::ResizeHandle;

use crate::systems::input::gestures::Gesture;

/// Claims a press that lands on the HUD so the world ignores it; bevy's picking drives the actual drag,
/// so there is nothing to do once it is claimed. Its cursor marks the hovered surface as draggable, or
/// as resizable over a resize grip.
pub struct DragGesture;

inventory::submit! {
    &DragGesture as &dyn Gesture
}

impl Gesture for DragGesture {
    fn priority(&self) -> i32 {
        0
    }

    fn claims(&self, world: &mut World) -> bool {
        hovered_has::<Node>(world)
    }

    fn drive(&self, _world: &mut World, _start: bool) {}

    fn cursor(&self, world: &mut World) -> Option<CursorIcon> {
        let icon = if hovered_has::<ResizeHandle>(world) {
            SystemCursorIcon::NwseResize
        } else {
            SystemCursorIcon::Pointer
        };
        Some(CursorIcon::System(icon))
    }
}

/// Whether any entity under the pointer carries component `C`.
fn hovered_has<C: Component>(world: &World) -> bool {
    world
        .resource::<HoverMap>()
        .values()
        .flat_map(|hits| hits.keys())
        .any(|&entity| world.get::<C>(entity).is_some())
}
