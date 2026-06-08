use rift::{Builder, Ctx};

use crate::core::protocol::{ACTION_DEAD, ACTION_IDLE, Actor, is_dead, set_action};

// Runs first: every actor starts the tick from a blank action; later systems overwrite it.
pub fn feature(b: &mut Builder) {
    b.system(reset);
}

fn reset(ctx: &mut Ctx) {
    let world = &mut ctx.server.world;
    for entity in world.ids::<Actor>() {
        let action = if is_dead(world, entity) {
            ACTION_DEAD
        } else {
            ACTION_IDLE
        };
        set_action(world, entity, action);
    }
}
