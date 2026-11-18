use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::script::{
    Compiler, Eval, Expr, QueryDef, ScriptFault, Value, ValueType, Vocabulary,
};
use crate::core::text::LineText;
use crate::systems::aim::character;
use crate::systems::mount;
use crate::systems::requirement::Describers;

/// Whether the character sits on a mount.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct Riding {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub of: Option<Box<Expr>>,
}

impl QueryDef for Riding {
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
        let Riding { of } = self;
        Ok({
            let of = cx.actor(of.as_deref())?;
            Box::new(move |world, cx| {
                Value::Bool(character(world, cx, &of).is_some_and(|who| mount::rides(world, who)))
            })
        })
    }
}

pub fn scripting(vocabulary: &mut Vocabulary) {
    vocabulary.query::<Riding>("riding");
    vocabulary
        .extension::<Describers>()
        .alone::<Riding>(|_q, d| LineText::plain(d.words().riding()));
}
