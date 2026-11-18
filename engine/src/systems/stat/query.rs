use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::script::{
    Compiler, Eval, Expr, QueryDef, ScriptFault, Value, ValueType, Vocabulary,
};
use crate::core::text::LineText;
use crate::systems::aim::actor;
use crate::systems::requirement::Describers;
use crate::systems::stat;
use crate::systems::stat::StatKind;

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct Stat {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub of: Option<Box<Expr>>,
    pub stat: StatKind,
}

impl QueryDef for Stat {
    fn static_type(&self) -> Option<ValueType> {
        Some(ValueType::Number)
    }

    fn reads_subject(&self) -> bool {
        self.of.is_none()
    }

    fn is_shared(&self) -> bool {
        false
    }

    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<Eval, ScriptFault> {
        let Stat { of, stat } = self;
        Ok({
            let stat = *stat;
            let of = cx.actor(of.as_deref())?;
            Box::new(move |world, cx| {
                Value::Num(
                    actor(world, cx, &of)
                        .map_or(0.0, |who| f64::from(stat::effective(world, who, stat))),
                )
            })
        })
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct Health {
    #[schemars(extend("default" = {"type": "subject"}))]
    pub actor: Box<Expr>,
}

impl QueryDef for Health {
    fn static_type(&self) -> Option<ValueType> {
        Some(ValueType::Number)
    }

    fn reads_subject(&self) -> bool {
        false
    }

    fn is_shared(&self) -> bool {
        false
    }

    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<Eval, ScriptFault> {
        let Health { actor: who } = self;
        Ok({
            let who = cx.expr(who, ValueType::Actor)?;
            Box::new(move |world, cx| {
                Value::Num(
                    actor(world, cx, &who)
                        .map_or(0.0, |who| f64::from(stat::current_health(world, who))),
                )
            })
        })
    }
}

pub fn scripting(vocabulary: &mut Vocabulary) {
    vocabulary.query::<Stat>("stat").query::<Health>("health");
    vocabulary
        .extension::<Describers>()
        .alone::<Stat>(|q, d| LineText::plain(q.stat.named(&d.messages.stat)))
        .compared::<Stat>(|q, limit, d| match limit {
            Expr::Num(min) => Some(d.words().stat_of(*min, q.stat.named(&d.messages.stat))),
            Expr::Int(min) => Some(d.words().stat_of(*min, q.stat.named(&d.messages.stat))),
            _ => None,
        })
        .alone::<Health>(|_q, d| LineText::plain(d.words().health()));
}
