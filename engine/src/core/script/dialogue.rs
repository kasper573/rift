use crate::core::content::SchemaMark;
use std::collections::HashSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{IntoStaticStr, VariantArray};

use super::expr::Expr;
use super::{Arrow, ChoiceLabel, DialogueLine, ScriptEffect, ScriptFault, Value};

pub const MAX_LINES: usize = 16;
pub const MAX_CHOICES: usize = 12;

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DialogueNode {
    pub lines: Vec<DialogueLine>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<Choice>,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub next: Arrow,
    #[schemars(transform = SchemaMark::Fails.with(true))]
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub left: Arrow,
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Choice {
    pub label: ChoiceLabel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires: Option<Expr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reveal: Vec<ChoiceReveal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warn: Option<String>,
    #[serde(default, skip_serializing_if = "Arrow::is_none")]
    pub to: Arrow,
}

/// What a choice shows of what it asks or gives, and whether it shows locked while its requirements
/// are unmet.
#[derive(
    JsonSchema,
    Serialize,
    Deserialize,
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Hash,
    IntoStaticStr,
    VariantArray,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ChoiceReveal {
    Locked,
    Needs,
    Costs,
    Gains,
}

impl ChoiceReveal {
    /// Whether a choice revealing this still shows, locked, while its requirements are unmet.
    pub fn shows_while_locked(self) -> bool {
        self == ChoiceReveal::Locked
    }

    pub fn name(self) -> &'static str {
        self.into()
    }
}

/// How a script's dialogue is seen.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum DialogueCamera {
    /// From behind whoever is spoken to, over their shoulder, onto whoever speaks.
    #[default]
    Shoulder,
    /// Everyone in the conversation at once, from as far round the player as the game's shot
    /// stands: in front of them, or beside them for a profile of everyone.
    Participants,
    /// Where the player keeps the camera.
    Gameplay,
}

pub fn check_reveals_unique(reveal: &[ChoiceReveal]) -> Result<(), ScriptFault> {
    let mut seen = HashSet::new();
    match reveal.iter().find(|kind| !seen.insert(**kind)) {
        Some(kind) => Err(ScriptFault::new(
            "revealed twice",
            format!("{} twice", kind.name()),
        )),
        None => Ok(()),
    }
}

pub struct DialogueView<'a> {
    pub script: &'a str,
    pub camera: DialogueCamera,
    pub node: &'a str,
    pub lines: Vec<(&'a DialogueLine, Vec<Value>)>,
    pub choices: Vec<ChoiceOffer<'a>>,
}

pub struct ChoiceOffer<'a> {
    pub index: usize,
    pub label: &'a ChoiceLabel,
    pub label_values: Vec<Value>,
    pub locked: bool,
    pub requires: Option<&'a Expr>,
    pub reveal: &'a [ChoiceReveal],
    pub warn: Option<&'a str>,
    pub leads: Vec<&'a ScriptEffect>,
}
