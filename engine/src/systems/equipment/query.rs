use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::script::{
    Compiler, Eval, Expr, QueryDef, ScriptFault, Value, ValueType, Vocabulary,
};
use crate::systems::aim::character;
use crate::systems::equipment;
use crate::systems::item::ItemId;
use crate::systems::mention::Mention;
use crate::systems::requirement::Describers;

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct Equipped {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub of: Option<Box<Expr>>,
    pub item: ItemId,
}

impl QueryDef for Equipped {
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
        let Equipped { of, item } = self;
        Ok({
            let item = *item;
            let of = cx.actor(of.as_deref())?;
            Box::new(move |world, cx| {
                Value::Bool(
                    character(world, cx, &of).is_some_and(|who| equipment::wears(world, who, item)),
                )
            })
        })
    }
}

pub fn scripting(vocabulary: &mut Vocabulary) {
    vocabulary.query::<Equipped>("equipped");
    vocabulary
        .extension::<Describers>()
        .alone::<Equipped>(|q, d| d.words().worn(d.names.mentioned(Mention::Item(q.item))));
}
