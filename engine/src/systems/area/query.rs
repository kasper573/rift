use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::script::{
    Compiler, Eval, Expr, QueryDef, ScriptFault, Value, ValueType, Vocabulary,
};
use crate::systems::aim::{actor, character};
use crate::systems::area::AreaId;
use crate::systems::area::AreaTag;
use crate::systems::area::object;
use crate::systems::area::transition::Discovered;
use crate::systems::mention::Mention;
use crate::systems::requirement::Describers;

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct Inside {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub of: Option<Box<Expr>>,
    pub object: String,
}

impl QueryDef for Inside {
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
        let Inside { of, object } = self;
        Ok({
            let object = object.clone();
            let of = cx.actor(of.as_deref())?;
            Box::new(move |world, cx| {
                Value::Bool(
                    actor(world, cx, &of).is_some_and(|who| object::inside(world, who, &object)),
                )
            })
        })
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct InArea {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub of: Option<Box<Expr>>,
    pub area: AreaId,
}

impl QueryDef for InArea {
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
        let InArea { of, area } = self;
        Ok({
            let area = *area;
            let of = cx.actor(of.as_deref())?;
            Box::new(move |world, cx| {
                Value::Bool(actor(world, cx, &of).is_some_and(|who| {
                    world
                        .get::<AreaTag>(who)
                        .is_some_and(|tag| tag.area == area)
                }))
            })
        })
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct KnowsArea {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub of: Option<Box<Expr>>,
    pub area: AreaId,
}

impl QueryDef for KnowsArea {
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
        let KnowsArea { of, area } = self;
        Ok({
            let area = *area;
            let of = cx.actor(of.as_deref())?;
            Box::new(move |world, cx| {
                Value::Bool(character(world, cx, &of).is_some_and(|who| {
                    world
                        .get::<Discovered>(who)
                        .is_some_and(|known| known.knows(area))
                }))
            })
        })
    }
}

pub fn scripting(vocabulary: &mut Vocabulary) {
    vocabulary
        .query::<Inside>("inside")
        .query::<InArea>("in_area")
        .query::<KnowsArea>("knows_area");
    vocabulary
        .extension::<Describers>()
        .alone::<Inside>(|q, d| d.words().standing_on(q.object.to_string()))
        .alone::<InArea>(|q, d| d.words().in_area(d.names.mentioned(Mention::Area(q.area))))
        .alone::<KnowsArea>(|q, d| {
            d.words()
                .knows_area(d.names.mentioned(Mention::Area(q.area)))
        });
}
