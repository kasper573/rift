use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::script::{
    Compiler, Eval, Expr, QueryDef, ScriptFault, Value, ValueType, Vocabulary,
};
use crate::core::text::LineText;
use crate::systems::aim::character;
use crate::systems::class;
use crate::systems::class::ClassId;
use crate::systems::requirement::Describers;

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct Level {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub of: Option<Box<Expr>>,
}

impl QueryDef for Level {
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
        let Level { of } = self;
        Ok({
            let of = cx.actor(of.as_deref())?;
            Box::new(move |world, cx| {
                Value::Int(
                    character(world, cx, &of).map_or(0, |who| i64::from(class::level(world, who))),
                )
            })
        })
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct IsClass {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub of: Option<Box<Expr>>,
    pub class: ClassId,
}

impl QueryDef for IsClass {
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
        let IsClass { of, class } = self;
        Ok({
            let class = *class;
            let of = cx.actor(of.as_deref())?;
            Box::new(move |world, cx| {
                Value::Bool(
                    character(world, cx, &of)
                        .is_some_and(|who| class::of(world, who) == Some(class)),
                )
            })
        })
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct SpentSkillPoints {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub of: Option<Box<Expr>>,
}

impl QueryDef for SpentSkillPoints {
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
        let SpentSkillPoints { of } = self;
        Ok({
            let of = cx.actor(of.as_deref())?;
            Box::new(move |world, cx| {
                Value::Int(
                    character(world, cx, &of)
                        .map_or(0, |who| i64::from(class::spent_points(world, who).0)),
                )
            })
        })
    }
}

pub fn scripting(vocabulary: &mut Vocabulary) {
    vocabulary
        .query::<Level>("level")
        .query::<IsClass>("is_class")
        .query::<SpentSkillPoints>("spent_skill_points");
    vocabulary
        .extension::<Describers>()
        .alone::<Level>(|_q, d| LineText::plain(d.words().level()))
        .compared::<Level>(|_q, limit, d| match limit {
            Expr::Int(level) => Some(d.words().level_of(*level)),
            _ => None,
        })
        .alone::<IsClass>(|q, d| d.words().is_class(d.names.class(q.class)))
        .alone::<SpentSkillPoints>(|_q, d| LineText::plain(d.words().skill_points_spent()));
}
