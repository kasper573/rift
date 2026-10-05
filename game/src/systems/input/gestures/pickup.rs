use crate::systems::item;
use crate::systems::player::session;
use bevy::prelude::*;
use bevy::window::CursorIcon;

use crate::core::render;
use crate::systems::input::gestures::{Gesture, image_cursor};

pub struct PickupGesture;

impl Gesture for PickupGesture {
    fn priority(&self) -> i32 {
        3
    }

    fn claims(&self, world: &mut World) -> bool {
        session::is_alive(world)
            && render::cursor_tile(world)
                .and_then(|point| item::pickable_at(world, point))
                .is_some()
    }

    fn drive(&self, world: &mut World, start: bool) {
        if start
            && let Some(point) = render::cursor_tile(world)
            && let Some(target) = item::pickable_at(world, point)
        {
            session::pickup(world, target);
        }
    }

    fn cursor(&self, world: &mut World) -> Option<CursorIcon> {
        let handle = world
            .resource::<AssetServer>()
            .load("icons/cursors/hand001.png");
        Some(image_cursor(handle, (8, 8)))
    }
}
