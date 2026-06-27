use super::{Effect, EffectContext, encode_args};
use crate::systems::stat::{self, MovementSpeedStat, StatSet};

/// An npc moves this much faster while it has a target.
const CHASE_SPEED_MULTIPLIER: f32 = 2.0;

/// Raised on an npc while it chases (see `npc::chase`); takes no args and reads the chaser's base
/// speed from the context, so the boost scales with each npc.
pub struct Chasing;

inventory::submit! {
    &Chasing as &dyn Effect
}

impl Effect for Chasing {
    fn name(&self) -> &str {
        "Chasing"
    }
    fn icon(&self) -> Option<&str> {
        None
    }
    fn encode(&self, args: serde_json::Value) -> Result<Vec<u8>, String> {
        encode_args::<()>(args)
    }
    fn compute(&self, ctx: &EffectContext, _args: &[u8]) -> StatSet {
        let base = stat::base(ctx.world, ctx.source, MovementSpeedStat.into());
        StatSet::single(
            MovementSpeedStat.into(),
            base * (CHASE_SPEED_MULTIPLIER - 1.0),
        )
    }
}
