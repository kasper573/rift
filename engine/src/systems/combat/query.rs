use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::script::{
    Compiler, Eval, Expr, QueryDef, ScriptFault, Value, ValueType, Vocabulary,
};
use crate::core::text::LineText;
use crate::systems::aim::actor;
use crate::systems::combat;
use crate::systems::combat::Attitude;
use crate::systems::requirement::Describers;

/// Whether the character struck someone or was struck within the time the game's claims
/// last.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct InCombat {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub of: Option<Box<Expr>>,
}

impl QueryDef for InCombat {
    fn static_type(&self) -> Option<ValueType> {
        Some(ValueType::Boolean)
    }

    fn reads_subject(&self) -> bool {
        self.of.is_none()
    }

    fn is_shared(&self) -> bool {
        false
    }

    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<Eval, ScriptFault> {
        let InCombat { of } = self;
        Ok({
            let of = cx.actor(of.as_deref())?;
            Box::new(move |world, cx| {
                Value::Bool(actor(world, cx, &of).is_some_and(|who| combat::in_combat(world, who)))
            })
        })
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct Hostile {
    #[schemars(extend("default" = {"type": "subject"}))]
    pub actor: Box<Expr>,
}

impl QueryDef for Hostile {
    fn static_type(&self) -> Option<ValueType> {
        Some(ValueType::Boolean)
    }

    fn reads_subject(&self) -> bool {
        false
    }

    fn is_shared(&self) -> bool {
        false
    }

    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<Eval, ScriptFault> {
        let Hostile { actor: who } = self;
        Ok({
            let who = cx.expr(who, ValueType::Actor)?;
            Box::new(move |world, cx| {
                Value::Bool(
                    actor(world, cx, &who)
                        .is_some_and(|who| world.get::<Attitude>(who) == Some(&Attitude::Hostile)),
                )
            })
        })
    }
}

pub fn scripting(vocabulary: &mut Vocabulary) {
    vocabulary
        .query::<InCombat>("in_combat")
        .query::<Hostile>("hostile");
    vocabulary
        .extension::<Describers>()
        .alone::<InCombat>(|_q, d| LineText::plain(d.words().in_combat()))
        .alone::<Hostile>(|_q, d| LineText::plain(d.words().hostile()));
}
