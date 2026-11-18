use crate::core::script::Reach;
use crate::core::script::ReachDef;
use crate::core::script::Vocabulary;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::SchemaMark;
use crate::core::script::{Arrow, Compiler, EffectDef, EffectStep, NEXT, ScriptFault};
use crate::core::script::{EffectCall, Expr};
use crate::systems::aim as effects;

/// Opens the market for a player, in a window of its own: the script goes on at once. A market
/// opened by a thing in the world closes as the player walks out of its reach.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[schemars(transform = SchemaMark::System.with("market"))]
pub struct OpenMarket {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub who: Option<Expr>,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
}

impl EffectDef for OpenMarket {
    fn needs_subject(&self) -> bool {
        self.who.is_none()
    }
    crate::outputs!(NEXT => next);
    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<EffectCall, ScriptFault> {
        let who = effects::aim(cx, &self.who)?;
        Ok(Box::new(move |world, _, cx| {
            if let Some(player) = effects::aimed_character(world, cx, &who) {
                super::open(world, player, cx.owner);
            }
            EffectStep::Done
        }))
    }
    fn reach(&self, _cx: &Compiler) -> Vec<Reach> {
        vec![Reach::new(OpensMarket)]
    }
}

/// A script that may open the market.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OpensMarket;

impl ReachDef for OpensMarket {}

pub fn scripting(vocabulary: &mut Vocabulary) {
    vocabulary.effect::<OpenMarket>("open_market");
}
