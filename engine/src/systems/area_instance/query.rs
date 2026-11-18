use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::script::{
    Compiler, Eval, Expr, QueryDef, ScriptFault, Value, ValueType, Vocabulary,
};
use crate::core::text::LineText;
use crate::systems::aim::character;
use crate::systems::area_instance;
use crate::systems::requirement::Describers;

/// Whether the character stands in an instance.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct InInstance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub of: Option<Box<Expr>>,
}

impl QueryDef for InInstance {
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
        let InInstance { of } = self;
        Ok({
            let of = cx.actor(of.as_deref())?;
            Box::new(move |world, cx| {
                Value::Bool(character(world, cx, &of).is_some() && area_instance::inside(world))
            })
        })
    }
}

pub fn scripting(vocabulary: &mut Vocabulary) {
    vocabulary.query::<InInstance>("in_instance");
    vocabulary
        .extension::<Describers>()
        .alone::<InInstance>(|_q, d| LineText::plain(d.words().in_instance()));
}
