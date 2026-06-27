use serde::{Deserialize, Serialize};

use super::{Effect, EffectContext, decode, encode_args};
use crate::systems::stat::{StatId, StatSet};

#[derive(Serialize, Deserialize)]
struct Args {
    stat: StatId,
    amount: f32,
}

pub struct StatModifier;

inventory::submit! {
    &StatModifier as &dyn Effect
}

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
