use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::script::{
    Compiler, Eval, Expr, QueryDef, ScriptFault, Value, ValueType, Vocabulary,
};
use crate::core::text::LineText;
use crate::core::units::Seconds;
use crate::systems::aim::character;
use crate::systems::memory;
use crate::systems::memory::MemoryId;
use crate::systems::requirement::Describers;

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct Memory {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub of: Option<Box<Expr>>,
    pub memory: MemoryId,
}

impl QueryDef for Memory {
    fn static_type(&self) -> Option<ValueType> {
        None
    }

    fn value_type(&self, cx: &Compiler) -> ValueType {
        self.memory.get(cx.content).ty
    }

    fn reads_subject(&self) -> bool {
        self.of.is_none()
    }

    fn is_shared(&self) -> bool {
        false
    }

    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<Eval, ScriptFault> {
        let Memory { of, memory } = self;
        Ok({
            let memory = *memory;
            let of = cx.actor(of.as_deref())?;
            let ty = memory.get(cx.content).ty;
            Box::new(move |world, cx| {
                character(world, cx, &of)
                    .map_or_else(|| ty.zero(), |who| memory::get(world, who, memory))
            })
        })
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct SetWithin {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub of: Option<Box<Expr>>,
    pub memory: MemoryId,
    #[schemars(extend("default" = 60.0))]
    pub seconds: Box<Expr>,
}

impl QueryDef for SetWithin {
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
        let SetWithin {
            of,
            memory,
            seconds,
        } = self;
        Ok({
            let memory = *memory;
            let of = cx.actor(of.as_deref())?;
            let seconds = cx.expr(seconds, ValueType::Number)?;
            Box::new(move |world, cx| {
                Value::Bool(character(world, cx, &of).is_some_and(|who| {
                    memory::set_within(
                        world,
                        who,
                        memory,
                        Seconds(seconds(world, cx).as_num() as f32),
                    )
                }))
            })
        })
    }
}

pub fn scripting(vocabulary: &mut Vocabulary) {
    vocabulary
        .query::<Memory>("memory")
        .query::<SetWithin>("set_within");
    vocabulary
        .extension::<Describers>()
        .alone::<Memory>(|q, d| LineText::plain(d.names.memory(q.memory)))
        .compared::<Memory>(|q, limit, d| match limit {
            Expr::Int(count) => Some(d.words().memory_of(*count, d.names.memory(q.memory))),
            _ => None,
        })
        .alone::<SetWithin>(|q, d| d.words().set_lately(d.names.memory(q.memory)));
}
