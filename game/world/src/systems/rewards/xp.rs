use serde::Deserialize;

use super::{Grant, GrantCtx};
use crate::systems::player::Xp as Experience;

/// Grants experience to the player who reserved the kill.
#[derive(Deserialize)]
pub struct Xp {}

impl Grant for Xp {
    fn grant(&self, ctx: &mut GrantCtx) {
        if let Some(entity) = ctx.rewardee
            && let Some(mut xp) = ctx.world.get_mut::<Experience>(entity)
        {
            xp.gain(ctx.amount);
        }
    }
}
