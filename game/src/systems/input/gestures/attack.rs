use crate::systems::combat;
use crate::systems::player::session;
use bevy::prelude::*;
use bevy::window::CursorIcon;

use crate::core::render;
use crate::systems::input::gestures::{Gesture, content_cursor};
use crate::systems::interface::CursorShape;

pub struct AttackGesture;

impl Gesture for AttackGesture {
    fn priority(&self) -> i32 {
        2
    }

    fn claims(&self, world: &mut World) -> bool {
        session::is_alive(world)
            && render::cursor_tile(world)
                .is_some_and(|point| combat::enemy_at(world, point).is_some())
    }

    fn drive(&self, world: &mut World, start: bool) {
        if start
            && let Some(point) = render::cursor_tile(world)
            && let Some(target) = combat::enemy_at(world, point)
        {
            session::attack(world, target);
        }
    }

    fn cursor(&self, world: &mut World) -> Option<CursorIcon> {
        Some(content_cursor(world, CursorShape::Attack))
    }
}
