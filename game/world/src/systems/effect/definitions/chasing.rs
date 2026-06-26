use super::{Effect, EffectCategory};
use crate::systems::effect::EffectContext;
use crate::systems::stat::{self, MovementSpeedStat, StatSet};

/// An npc moves this much faster while it has a target.
const CHASE_SPEED_MULTIPLIER: f32 = 2.0;

/// Raised on an npc while it chases (see `npc::chase`); reads the chaser's base speed from the
/// context rather than taking args, so the boost scales with each npc.
pub struct Chasing;

impl Effect for Chasing {
    type Args = ();

    fn name(&self) -> &str {
        "Chasing"
    }
    fn category(&self) -> EffectCategory {
        EffectCategory::Neutral
    }
    fn icon(&self) -> Option<&str> {
        None
    }
    fn compute(&self, ctx: &EffectContext, _args: ()) -> StatSet {
        let base = stat::base(ctx.world, ctx.source, MovementSpeedStat.into());
        StatSet::single(
            MovementSpeedStat.into(),
            base * (CHASE_SPEED_MULTIPLIER - 1.0),
        )
    }
    fn describe(&self, ctx: &EffectContext, args: ()) -> String {
        self.compute(ctx, args).describe()
    }
}
