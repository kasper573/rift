//! Actor systems: settle every actor's action back to idle/dead each tick (movement and combat
//! re-assert walk/run/attack afterwards), plus the action mutators the other systems use.

use bevy_ecs::prelude::*;
use world::actor::{ACTION_DEAD, ACTION_IDLE, Actor};
use world::combat::Vitals;

pub fn set_action(actor: &mut Mut<Actor>, action: u8) {
    if actor.action != action {
        actor.action = action;
    }
}

pub fn set_facing(actor: &mut Mut<Actor>, dir: u8, action: u8) {
    if actor.dir != dir || actor.action != action {
        actor.dir = dir;
        actor.action = action;
    }
}

pub fn reset(mut actors: Query<(&mut Actor, Option<&Vitals>)>) {
    for (mut actor, vitals) in &mut actors {
        let dead = vitals.is_some_and(|v| v.health <= 0.0);
        set_action(&mut actor, if dead { ACTION_DEAD } else { ACTION_IDLE });
    }
}
