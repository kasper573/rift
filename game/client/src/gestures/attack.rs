use bevy::prelude::*;
use world::query;
use world::session;

use crate::gestures::InputIntent;
use crate::render;

pub(super) struct Attack;

impl InputIntent for Attack {
    fn claims(&self, world: &mut World) -> bool {
        !session::is_dead(world)
            && render::cursor_tile(world)
                .is_some_and(|point| query::enemy_at(world, point).is_some())
    }

    fn drive(&self, world: &mut World, start: bool) {
        if start
            && let Some(point) = render::cursor_tile(world)
            && let Some(target) = query::enemy_at(world, point)
        {
            session::attack(world, target);
        }
    }
}
