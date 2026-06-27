use super::{Effect, EffectContext, encode_args};
use crate::systems::stat::{self, MovementSpeedStat, StatSet};

const CHASE_SPEED_MULTIPLIER: f32 = 2.0;

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
