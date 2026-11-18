pub mod bind;
pub mod compile;
pub mod condition;
pub mod dialogue;
mod effect;
mod event;
pub mod expr;
mod kinds;
pub mod may_do;
pub mod node;
mod query;
mod reach;
pub mod run;
mod runner;
pub mod value;
pub mod vocabulary;
mod words;

pub use bind::{
    Bind, Binder, ScriptSite, ScriptedRow, owner_label, owner_name, owner_site, spawn_world_owners,
};
pub use compile::{
    Answered, Compiler, EffectCall, EffectLook, EffectStep, Eval, OwnerRow, Procedure, Reached,
    RowValue, Scripts, Step,
};
pub use dialogue::{
    Choice, ChoiceOffer, ChoiceReveal, DialogueCamera, DialogueNode, DialogueView,
    check_reveals_unique,
};
pub use effect::{EffectDef, ScriptEffect};
pub use event::{EventDef, EventKind, Run, ScriptEvent};
pub use expr::{Expr, ExprKind};
pub use may_do::MayDo;
pub use node::{Assignment, Exit, Node, NodeKind, Target};
pub use query::{QueryDef, ScriptQuery};
pub use reach::{Reach, ReachDef};
pub use run::{
    Answer, BoxEnding, ChoiceState, FlowCx, FlowKey, FlowSteering, FlowTrace, Instances, Locals,
    Owner, Pause, TraceStep,
};
pub use runner::{RunnerKind, RunnerSpec};
pub use value::{GlobalPlayerId, Value, ValueType};
pub use vocabulary::{DialogueHost, PlayerHost, Vocabulary};
pub use words::{ChoiceLabel, DialogueLine, PlaceDef, PlaceValue, WordsDef};

use std::collections::BTreeMap;
use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{EnumString, IntoStaticStr, VariantArray};

use crate::core::content::SchemaMark;
use crate::core::content::{ContentFile, ContentId, Nearby};

/// The output an effect takes when nothing else is said.
pub const NEXT: &str = "next";
/// The output an effect takes when it would not do what it was asked.
pub const REFUSED: &str = "refused";
/// The output an effect takes once what it was asked is done.
pub const DONE: &str = "done";
/// The output an effect takes when its subject accepts what it offered.
pub const ACCEPTED: &str = "accepted";
/// The output an effect takes when its subject declines what it offered.
pub const DECLINED: &str = "declined";
/// The output an effect takes when the window it opened is closed.
pub const CLOSED: &str = "closed";

#[derive(JsonSchema, Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(transparent)]
#[schemars(transform = SchemaMark::Arrow.with(true))]
pub struct Arrow(Option<String>);

impl Arrow {
    pub fn to(&self) -> Option<&str> {
        self.0.as_deref()
    }

    pub fn is_none(&self) -> bool {
        self.0.is_none()
    }

    /// Points the arrow at the node named `to`, or at nothing.
    pub fn point(&mut self, to: Option<&str>) {
        self.0 = to.map(str::to_owned);
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptDef {
    pub runs_on: RunnerKind,
    /// How the script's dialogue is seen, unless a line says otherwise.
    #[serde(default, skip_serializing_if = "is_default")]
    pub camera: DialogueCamera,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub variables: BTreeMap<String, Variable>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub state: BTreeMap<String, ValueType>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub locals: BTreeMap<String, ValueType>,
    pub nodes: BTreeMap<String, Node>,
}

fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

/// Whether `name` may name a node of a script: letters, digits and underscores, at least one.
pub fn valid_node_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

impl ScriptDef {
    /// A script with no nodes yet, run by owners of `kind`.
    pub fn running_on(kind: RunnerKind) -> ScriptDef {
        ScriptDef {
            runs_on: kind,
            camera: DialogueCamera::default(),
            variables: BTreeMap::new(),
            state: BTreeMap::new(),
            locals: BTreeMap::new(),
            nodes: BTreeMap::new(),
        }
    }
}

impl ContentFile for ScriptDef {
    const TABLE: &'static str = "script";

    fn fresh(_: &Nearby) -> Result<ScriptDef, String> {
        Ok(ScriptDef::running_on(RunnerKind::ANY))
    }
}

pub type ScriptId = ContentId<ScriptDef>;

/// A script row run by the row holding the module, with values for the script's variables.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptModule {
    pub script: ScriptId,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, VarValue>,
}

impl ScriptModule {
    /// The `type` a script module is tagged with in a row's list of modules.
    pub const TAG: &'static str = "script";
}

/// A script module as a row lists it among its modules, tagged with its `type`.
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ListedScriptModule {
    Script(ScriptModule),
}

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
    EnumString,
    IntoStaticStr,
    VariantArray,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum VariableType {
    Integer,
    Number,
    Boolean,
    Text,
    Name,
    Place,
    Row,
    Audio,
    Condition,
    /// A player in the script's own world: the one named, or the actor an expression finds.
    LocalPlayer,
    /// A player wherever on the server they play: the one named, or the one whose character an
    /// expression finds.
    GlobalPlayer,
}

impl VariableType {
    pub fn name(self) -> &'static str {
        self.into()
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Variable {
    #[serde(rename = "type")]
    pub ty: VariableType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<VarValue>,
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum VarValue {
    Integer(i64),
    Number(f64),
    Boolean(bool),
    Str(String),
    Place(PlaceValue),
    Condition(Expr),
}

impl VarValue {
    pub fn kind(&self) -> &'static str {
        match self {
            VarValue::Integer(_) => "an integer",
            VarValue::Number(_) => "a number",
            VarValue::Boolean(_) => "a boolean",
            VarValue::Str(_) => "a string",
            VarValue::Place(_) => "a place",
            VarValue::Condition(_) => "a condition",
        }
    }
}

#[derive(JsonSchema, Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Fill<T> {
    Var { var: String },
    Const(T),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptFault {
    pub rule: &'static str,
    pub expected: String,
}

impl ScriptFault {
    pub fn new(rule: &'static str, expected: impl fmt::Display) -> ScriptFault {
        ScriptFault {
            rule,
            expected: expected.to_string(),
        }
    }
}

impl fmt::Display for ScriptFault {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}: {}", self.rule, self.expected)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptError {
    pub rule: &'static str,
    pub owner: String,
    pub node: Option<String>,
    pub expected: String,
    pub script: Option<String>,
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}: {}", self.owner, self.rule)?;
        if let Some(node) = &self.node {
            write!(f, " at {node}")?;
        }
        write!(f, ": {}", self.expected)
    }
}
