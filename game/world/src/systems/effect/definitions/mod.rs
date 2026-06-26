//! Effect definitions: each is a unit struct in a file here implementing the typed [`Effect`] trait
//! (its own `Args`, its own `compute`). [`EffectKind`] is the `enum_dispatch` enum over them; because
//! `Effect` has an associated `Args` it can't be dispatched directly, so a blanket bridge erases the
//! args behind [`Definition`] (json/postcard) and that is what `EffectKind` dispatches.

mod chasing;
mod stat_modifier;

pub use chasing::Chasing;
pub use stat_modifier::StatModifier;

use enum_dispatch::enum_dispatch;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::EffectContext;
use crate::systems::stat::StatSet;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EffectCategory {
    Buff,
    Debuff,
    Neutral,
}

/// The interface every effect file implements: typed args, plus the immutable [`StatSet`] it
/// contributes and how it describes itself. It only reads `ctx`; applying the stats is the stats
/// system's job.
pub trait Effect {
    type Args: Serialize + DeserializeOwned;

    fn name(&self) -> &str;
    fn category(&self) -> EffectCategory;
    fn icon(&self) -> Option<&str>;
    fn compute(&self, ctx: &EffectContext, args: Self::Args) -> StatSet;
    fn describe(&self, ctx: &EffectContext, args: Self::Args) -> String;
}

/// Args-erased view of an [`Effect`], dispatched by [`EffectKind`]. Args cross this boundary as json
/// (table load) or postcard bytes (runtime/replication); a blanket impl bridges every [`Effect`].
#[enum_dispatch]
pub trait Definition {
    fn name(&self) -> &str;
    fn category(&self) -> EffectCategory;
    fn icon(&self) -> Option<&str>;
    fn encode(&self, args: serde_json::Value) -> Result<Vec<u8>, String>;
    fn compute(&self, ctx: &EffectContext, args: &[u8]) -> StatSet;
    fn describe(&self, ctx: &EffectContext, args: &[u8]) -> String;
}

impl<E: Effect> Definition for E {
    fn name(&self) -> &str {
        Effect::name(self)
    }
    fn category(&self) -> EffectCategory {
        Effect::category(self)
    }
    fn icon(&self) -> Option<&str> {
        Effect::icon(self)
    }
    fn encode(&self, args: serde_json::Value) -> Result<Vec<u8>, String> {
        let args: E::Args = serde_json::from_value(args).map_err(|error| error.to_string())?;
        postcard::to_allocvec(&args).map_err(|error| error.to_string())
    }
    fn compute(&self, ctx: &EffectContext, args: &[u8]) -> StatSet {
        Effect::compute(self, ctx, decode(args))
    }
    fn describe(&self, ctx: &EffectContext, args: &[u8]) -> String {
        Effect::describe(self, ctx, decode(args))
    }
}

fn decode<A: DeserializeOwned>(bytes: &[u8]) -> A {
    postcard::from_bytes(bytes).expect("args were validated and encoded at load")
}

/// Every effect, one variant per definition. `enum_dispatch` forwards [`Definition`] to the variant.
#[enum_dispatch(Definition)]
pub enum EffectKind {
    StatModifier(StatModifier),
    Chasing(Chasing),
}

impl EffectKind {
    pub fn all() -> Vec<EffectKind> {
        vec![StatModifier.into(), Chasing.into()]
    }
}
