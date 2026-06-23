use crate::session;
use bevy::prelude::*;
use bevy::window::CursorIcon;

use crate::input::gestures::{Gesture, image_cursor};
use crate::render;

/// Attack the actor under the cursor.
pub struct AttackGesture {
    attack: Handle<Image>,
}

impl AttackGesture {
    pub fn new(assets: &AssetServer) -> AttackGesture {
        AttackGesture {
            attack: assets.load("icons/cursors/swords002.png"),
        }
    }
}

impl Gesture for AttackGesture {
    fn claims(&self, world: &mut World) -> bool {
        !session::is_dead(world)
            && render::cursor_tile(world)
                .is_some_and(|point| session::enemy_at(world, point).is_some())
    }

    fn drive(&mut self, world: &mut World, start: bool) {
        if start
            && let Some(point) = render::cursor_tile(world)
            && let Some(target) = session::enemy_at(world, point)
        {
            session::attack(world, target);
        }
    }

    fn cursor(&self, _world: &mut World) -> Option<CursorIcon> {
        Some(image_cursor(self.attack.clone(), (32, 32)))
    }
}
