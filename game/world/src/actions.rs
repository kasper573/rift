use bevy_ecs::prelude::*;

use crate::protocol::{ACTION_DEAD, ACTION_IDLE, Actor, Vitals, set_action};

// Runs first: every actor starts the tick from a blank action; later systems overwrite it.
pub fn reset(mut actors: Query<(&mut Actor, Option<&Vitals>)>) {
    for (mut actor, vitals) in &mut actors {
        let dead = vitals.is_some_and(|v| v.health <= 0.0);
        set_action(&mut actor, if dead { ACTION_DEAD } else { ACTION_IDLE });
    }
}
