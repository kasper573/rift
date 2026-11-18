use crate::systems::prompt::ReadsOffer;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::script::{
    Compiler, Eval, Expr, QueryDef, Reach, ScriptFault, Value, ValueType, Vocabulary,
};
use crate::core::text::LineText;
use crate::systems::aim::character;
use crate::systems::item::ItemId;
use crate::systems::mention::Mention;
use crate::systems::prompt::Offer;
use crate::systems::requirement::Describers;

/// How many of `item` were last submitted to a prompt; how many items in all, without one.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct Offered {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub of: Option<Box<Expr>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<ItemId>,
}

impl QueryDef for Offered {
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
        let Offered { of, item } = self;
        Ok({
            let item = *item;
            let of = cx.actor(of.as_deref())?;
            cx.reaching(Reach::new(ReadsOffer));
            Box::new(move |world, cx| {
                Value::Int(character(world, cx, &of).map_or(0, |who| {
                    world
                        .get::<Offer>(who)
                        .map_or(0, |offer| i64::from(offer.count(item)))
                }))
            })
        })
    }
}

pub fn scripting(vocabulary: &mut Vocabulary) {
    vocabulary.query::<Offered>("offered");
    vocabulary
        .extension::<Describers>()
        .alone::<Offered>(|q, d| match q.item {
            Some(item) => d.words().offered(d.names.mentioned(Mention::Item(item))),
            None => LineText::plain(d.words().items_offered()),
        });
}
