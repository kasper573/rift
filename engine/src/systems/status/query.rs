use crate::systems::status::STATUS_EFFECT_RUNNER;
use crate::systems::status::StatusEffectDef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::Content;
use crate::core::script::{
    Compiler, Eval, Expr, QueryDef, ScriptFault, Value, ValueType, Vocabulary,
};
use crate::core::text::LineText;
use crate::systems::aim::actor;
use crate::systems::requirement::Describers;
use crate::systems::status;
use crate::systems::status::StatusCategory;
use crate::systems::status::StatusEffectId;

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct HasStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub of: Option<Box<Expr>>,
    pub effect: StatusEffectId,
}

impl QueryDef for HasStatus {
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
        let HasStatus { of, effect } = self;
        Ok({
            let effect = *effect;
            let of = cx.actor(of.as_deref())?;
            Box::new(move |world, cx| {
                Value::Bool(
                    actor(world, cx, &of).is_some_and(|who| status::has(world, who, effect)),
                )
            })
        })
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct StatusCount {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub of: Option<Box<Expr>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<StatusCategory>,
}

impl QueryDef for StatusCount {
    fn static_type(&self) -> Option<ValueType> {
        Some(ValueType::Integer)
    }

    fn reads_subject(&self) -> bool {
        self.of.is_none()
    }

    fn is_shared(&self) -> bool {
        false
    }

    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<Eval, ScriptFault> {
        let StatusCount { of, category } = self;
        Ok({
            let category = *category;
            let of = cx.actor(of.as_deref())?;
            Box::new(move |world, cx| {
                Value::Int(actor(world, cx, &of).map_or(0, |who| {
                    world.get::<status::Statuses>(who).map_or(0, |statuses| {
                        statuses.count(world.resource::<Content>(), category) as i64
                    })
                }))
            })
        })
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct StatusPower {}

impl QueryDef for StatusPower {
    fn static_type(&self) -> Option<ValueType> {
        Some(ValueType::Number)
    }

    fn reads_subject(&self) -> bool {
        true
    }

    fn is_shared(&self) -> bool {
        false
    }

    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<Eval, ScriptFault> {
        Ok({
            let effect = status_of(cx)?;
            Box::new(move |world, cx| {
                Value::Num(
                    cx.subject
                        .map_or(0.0, |who| f64::from(status::power(world, who, effect))),
                )
            })
        })
    }
}

pub fn scripting(vocabulary: &mut Vocabulary) {
    vocabulary
        .query::<HasStatus>("has_status")
        .query::<StatusCount>("status_count")
        .query::<StatusPower>("status_power");
    vocabulary
        .extension::<Describers>()
        .alone::<HasStatus>(|q, d| LineText::plain(d.names.status_effect(q.effect)))
        .alone::<StatusCount>(|q, d| {
            LineText::plain(match q.category {
                Some(category) => category.plural(&d.messages.status),
                None => d.messages.status.statuses(),
            })
        })
        .alone::<StatusPower>(|_q, d| LineText::plain(d.words().status_power()));
}

fn status_of(cx: &Compiler) -> Result<StatusEffectId, ScriptFault> {
    cx.owner_row()
        .filter(|row| row.kind == STATUS_EFFECT_RUNNER)
        .map(|row| row.id::<StatusEffectDef>())
        .ok_or_else(|| {
            ScriptFault::new(
                "wrong owner",
                "status_power reads the status whose script this is; only a status effect's script may ask",
            )
        })
}
