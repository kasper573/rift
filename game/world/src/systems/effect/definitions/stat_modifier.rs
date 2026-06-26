use serde::{Deserialize, Serialize};

use super::{Effect, EffectCategory};
use crate::systems::effect::EffectContext;
use crate::systems::stat::{StatKind, StatSet};

#[derive(Serialize, Deserialize)]
pub struct StatModifierArgs {
    pub stat: StatKind,
    pub amount: f32,
}

/// The catch-all effect for a flat stat delta. Every plain bonus or penalty uses this; effects that
/// need their own icon, scaling, or several stats get their own file instead.
pub struct StatModifier;

impl Effect for StatModifier {
    type Args = StatModifierArgs;

    fn name(&self) -> &str {
        "StatModifier"
    }
    fn category(&self) -> EffectCategory {
        EffectCategory::Neutral
    }
    fn icon(&self) -> Option<&str> {
        None
    }
    fn compute(&self, _ctx: &EffectContext, args: StatModifierArgs) -> StatSet {
        StatSet::single(args.stat, args.amount)
    }
    fn describe(&self, ctx: &EffectContext, args: StatModifierArgs) -> String {
        self.compute(ctx, args).describe()
    }
}
