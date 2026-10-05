use crate::systems::interact;
use crate::systems::player::session;
use bevy::prelude::*;
use bevy::window::CursorIcon;

use crate::core::render;
use crate::systems::input::gestures::{Gesture, image_cursor};

pub struct InteractGesture;

impl Gesture for InteractGesture {
    fn priority(&self) -> i32 {
        4
    }

    fn claims(&self, world: &mut World) -> bool {
        session::is_alive(world) && target(world).is_some()
    }

    fn drive(&self, world: &mut World, start: bool) {
        if start && let Some(target) = target(world) {
            session::interact(world, target);
        }
    }

    fn cursor(&self, world: &mut World) -> Option<CursorIcon> {
        let target = target(world)?;
        let verb = interact::interaction_of(world, target)?.verb;
        let handle = world.resource::<AssetServer>().load(verb.cursor());
        Some(image_cursor(handle, (4, 4)))
    }
}

fn target(world: &mut World) -> Option<Entity> {
    let point = render::cursor_tile(world)?;
    interact::interactable_at(world, point)
}
