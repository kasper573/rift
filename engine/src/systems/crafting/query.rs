use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::script::{
    Compiler, Eval, Expr, QueryDef, ScriptFault, Value, ValueType, Vocabulary,
};
use crate::systems::aim::character;
use crate::systems::crafting;
use crate::systems::crafting::RecipeKey;
use crate::systems::requirement::Describers;

/// Whether the character masters `recipe`: for good, or by a skill it knows.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct MastersRecipe {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"type": "subject"}))]
    pub of: Option<Box<Expr>>,
    pub recipe: RecipeKey,
}

impl QueryDef for MastersRecipe {
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
        let MastersRecipe { of, recipe } = self;
        Ok({
            crafting::mastery_fault(cx.content, recipe)?;
            let recipe = recipe.clone();
            let of = cx.actor(of.as_deref())?;
            Box::new(move |world, cx| {
                Value::Bool(
                    character(world, cx, &of)
                        .is_some_and(|who| crafting::masters(world, who, &recipe)),
                )
            })
        })
    }
}

pub fn scripting(vocabulary: &mut Vocabulary) {
    vocabulary.query::<MastersRecipe>("masters_recipe");
    vocabulary
        .extension::<Describers>()
        .alone::<MastersRecipe>(|q, d| d.words().mastery(q.recipe.recipe.0.clone()));
}
