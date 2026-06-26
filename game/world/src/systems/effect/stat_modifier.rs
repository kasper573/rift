use serde::{Deserialize, Serialize};

use super::{Effect, EffectContext, decode, encode_args};
use crate::systems::stat::{StatKind, StatSet};

#[derive(Serialize, Deserialize)]
struct Args {
    stat: StatKind,
    amount: f32,
}

/// The catch-all effect for a flat stat delta. Every plain bonus or penalty uses this; effects that
/// need their own icon, scaling, or several stats get their own file instead.
#[derive(Default)]
pub struct StatModifier;

impl Effect for StatModifier {
    fn name(&self) -> &str {
        "StatModifier"
    }
    fn icon(&self) -> Option<&str> {
        None
    }
    fn encode(&self, args: serde_json::Value) -> Result<Vec<u8>, String> {
        encode_args::<Args>(args)
    }
    fn compute(&self, _ctx: &EffectContext, args: &[u8]) -> StatSet {
        let args: Args = decode(args);
        StatSet::single(args.stat, args.amount)
    }
}
