use crate::core::content::SchemaMark;
use std::collections::HashSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::expr::Expr;
use super::{Arrow, RevealDef, ScriptFault, Value, Vocabulary};

pub const MAX_LINES: usize = 16;
pub const MAX_CHOICES: usize = 12;

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(bound = "", deny_unknown_fields)]
#[schemars(bound = "")]
pub struct DialogueNode<V: Vocabulary> {
    pub lines: Vec<V::Line>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<Choice<V>>,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
    #[schemars(transform = SchemaMark::Fails.with(true))]
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub left: Arrow,
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(bound = "", deny_unknown_fields)]
#[schemars(bound = "")]
pub struct Choice<V: Vocabulary> {
    pub label: V::Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires: Option<Expr<V>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reveal: Vec<V::Reveal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warn: Option<String>,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub to: Arrow,
}

pub fn check_reveals_unique<R: RevealDef>(reveal: &[R]) -> Result<(), ScriptFault> {
    let mut seen = HashSet::new();
    match reveal.iter().find(|kind| !seen.insert(**kind)) {
        Some(kind) => Err(ScriptFault::new(
            "revealed twice",
            format!("{} twice", kind.name()),
        )),
        None => Ok(()),
    }
}

pub struct DialogueView<'a, V: Vocabulary> {
    pub script: &'a str,
    pub node: &'a str,
    pub lines: Vec<(&'a V::Line, Vec<Value>)>,
    pub choices: Vec<ChoiceOffer<'a, V>>,
}

pub struct ChoiceOffer<'a, V: Vocabulary> {
    pub index: usize,
    pub label: &'a V::Text,
    pub label_values: Vec<Value>,
    pub locked: bool,
    pub requires: Option<&'a Expr<V>>,
    pub reveal: &'a [V::Reveal],
    pub warn: Option<&'a str>,
    pub leads: Vec<&'a V::Effect>,
}
