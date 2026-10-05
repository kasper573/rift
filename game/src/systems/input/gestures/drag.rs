use bevy::picking::hover::HoverMap;
use bevy::prelude::*;
use bevy::window::{CursorIcon, SystemCursorIcon};
use ui::ResizeHandle;

use crate::systems::actor::plate::WorldOverlay;
use crate::systems::input::gestures::Gesture;

pub struct DragGesture;

impl Gesture for DragGesture {
    fn priority(&self) -> i32 {
        0
    }

    fn claims(&self, world: &mut World) -> bool {
        over_interface(world)
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

pub(super) fn over_interface(world: &World) -> bool {
    hovered_has::<Node>(world)
}

fn hovered_has<C: Component>(world: &World) -> bool {
    world
        .resource::<HoverMap>()
        .values()
        .flat_map(|hits| hits.keys())
        .any(|&entity| world.get::<C>(entity).is_some() && !over_world(world, entity))
}

fn over_world(world: &World, entity: Entity) -> bool {
    std::iter::successors(Some(entity), |&entity| {
        world.get::<ChildOf>(entity).map(ChildOf::parent)
    })
    .any(|entity| world.get::<WorldOverlay>(entity).is_some())
}
