use serde::Deserialize;

use super::{Grant, GrantCtx};
use crate::systems::player::Xp;

#[derive(Deserialize)]
pub struct XpGrant {}

#[typetag::deserialize(name = "xp")]
impl Grant for XpGrant {
    fn grant(&self, ctx: &mut GrantCtx) {
        if let Some(entity) = ctx.rewardee
            && let Some(mut xp) = ctx.world.get_mut::<Xp>(entity)
        {
            xp.gain(ctx.amount);
        }
    }
}
