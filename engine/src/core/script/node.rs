use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{EnumDiscriminants, EnumString, IntoStaticStr, VariantArray};

use super::dialogue::DialogueNode;
use super::expr::Expr;
use super::{Arrow, EffectDef, ScriptId, VarValue, Vocabulary};
use crate::core::content::SchemaMark;
use crate::core::units::Seconds;

pub const MAX_NODES: usize = 2_000;
pub const MAX_SLOTS: usize = 32;
pub const MAX_REPEAT: u32 = 1_000;
pub const MAX_WAIT: Seconds = Seconds(3_600.0);
pub const MAX_STEPS: usize = 10_000;
pub const MAX_CALL_DEPTH: usize = 8;
pub const MAX_LIVE_FLOWS: usize = 32;

#[derive(JsonSchema, Clone, Serialize, Deserialize, EnumDiscriminants)]
#[serde(tag = "type", rename_all = "snake_case", bound = "")]
#[schemars(bound = "")]
#[strum_discriminants(
    name(NodeKind),
    derive(EnumString, IntoStaticStr, VariantArray),
    strum(serialize_all = "snake_case")
)]
pub enum Node<V: Vocabulary> {
    /// Starts a flow at an event; its cause is the subject.
    #[schemars(transform = SchemaMark::System.with("flow"))]
    On {
        #[serde(flatten)]
        event: V::Event,
        #[serde(default, skip_serializing_if = "Arrow::is_none")]
        next: Arrow,
    },
    /// Takes yes when the test holds, else no.
    #[schemars(transform = SchemaMark::System.with("flow"))]
    If {
        test: Expr<V>,
        #[serde(default, skip_serializing_if = "Arrow::is_none")]
        yes: Arrow,
        #[serde(default, skip_serializing_if = "Arrow::is_none")]
        no: Arrow,
    },
    /// Picks an output by weight.
    #[schemars(transform = SchemaMark::System.with("flow"))]
    Random {
        weights: Vec<u32>,
        outputs: Vec<Arrow>,
    },
    /// Runs each a number of times, then goes on.
    #[schemars(transform = SchemaMark::System.with("flow"))]
    Repeat {
        #[schemars(transform = SchemaMark::Count.with(true))]
        times: Expr<V>,
        at_most: u32,
        #[serde(default, skip_serializing_if = "Arrow::is_none")]
        each: Arrow,
        #[serde(default, skip_serializing_if = "Arrow::is_none")]
        then: Arrow,
    },
    /// Pauses for seconds, or one tick at 0.
    #[schemars(transform = SchemaMark::System.with("flow"))]
    Wait {
        seconds: Expr<V>,
        at_most: Seconds,
        #[serde(default, skip_serializing_if = "Arrow::is_none")]
        next: Arrow,
    },
    /// Writes locals and owner state in order.
    #[schemars(transform = SchemaMark::System.with("flow"))]
    Set {
        values: Vec<Assignment<V>>,
        #[serde(default, skip_serializing_if = "Arrow::is_none")]
        next: Arrow,
    },
    /// Calls another script and goes on when it ends.
    #[schemars(transform = SchemaMark::System.with("flow"))]
    Run {
        script: ScriptId<V>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        values: BTreeMap<String, VarValue<V>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        into: Option<String>,
        #[serde(default, skip_serializing_if = "Arrow::is_none")]
        next: Arrow,
    },
    /// Ends a called script, handing back a value.
    #[schemars(transform = SchemaMark::System.with("flow"))]
    Return {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<Expr<V>>,
    },
    /// Lines and choices spoken to the subject.
    #[schemars(transform = SchemaMark::System.with("dialogue"))]
    Dialogue(DialogueNode<V>),
    #[serde(untagged)]
    Effect(V::Effect),
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(bound = "")]
#[schemars(bound = "", transform = SchemaMark::Assignment.with(true))]
pub struct Assignment<V: Vocabulary> {
    #[serde(flatten)]
    pub target: Target,
    pub to: Expr<V>,
}

#[derive(JsonSchema, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    Local(String),
    State(String),
}

impl NodeKind {
    pub fn name(self) -> &'static str {
        self.into()
    }
}

/// Where an arrow leaves a node: an output it names, one of a random node's outputs, or the
/// arrow a dialogue choice takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Exit {
    Named(&'static str),
    Output(usize),
    Choice(usize),
}

pub const NEXT_EXIT: Exit = Exit::Named(super::NEXT);

impl<V: Vocabulary> Node<V> {
    pub fn kind(&self) -> NodeKind {
        self.into()
    }

    /// Every arrow leaving the node, where it leaves.
    pub fn exits(&self) -> Vec<(Exit, &Arrow)> {
        match self {
            Node::On { next, .. }
            | Node::Wait { next, .. }
            | Node::Set { next, .. }
            | Node::Run { next, .. } => vec![(NEXT_EXIT, next)],
            Node::If { yes, no, .. } => vec![(Exit::Named("yes"), yes), (Exit::Named("no"), no)],
            Node::Random { outputs, .. } => outputs
                .iter()
                .enumerate()
                .map(|(index, arrow)| (Exit::Output(index), arrow))
                .collect(),
            Node::Repeat { each, then, .. } => {
                vec![(Exit::Named("each"), each), (Exit::Named("then"), then)]
            }
            Node::Return { .. } => Vec::new(),
            Node::Dialogue(dialogue) => dialogue
                .choices
                .iter()
                .enumerate()
                .map(|(index, choice)| (Exit::Choice(index), &choice.to))
                .chain([
                    (NEXT_EXIT, &dialogue.next),
                    (Exit::Named("left"), &dialogue.left),
                ])
                .collect(),
            Node::Effect(effect) => effect
                .outputs()
                .into_iter()
                .map(|(name, arrow)| (Exit::Named(name), arrow))
                .collect(),
        }
    }

    pub fn exits_mut(&mut self) -> Vec<(Exit, &mut Arrow)> {
        match self {
            Node::On { next, .. }
            | Node::Wait { next, .. }
            | Node::Set { next, .. }
            | Node::Run { next, .. } => vec![(NEXT_EXIT, next)],
            Node::If { yes, no, .. } => vec![(Exit::Named("yes"), yes), (Exit::Named("no"), no)],
            Node::Random { outputs, .. } => outputs
                .iter_mut()
                .enumerate()
                .map(|(index, arrow)| (Exit::Output(index), arrow))
                .collect(),
            Node::Repeat { each, then, .. } => {
                vec![(Exit::Named("each"), each), (Exit::Named("then"), then)]
            }
            Node::Return { .. } => Vec::new(),
            Node::Dialogue(dialogue) => dialogue
                .choices
                .iter_mut()
                .enumerate()
                .map(|(index, choice)| (Exit::Choice(index), &mut choice.to))
                .chain([
                    (NEXT_EXIT, &mut dialogue.next),
                    (Exit::Named("left"), &mut dialogue.left),
                ])
                .collect(),
            Node::Effect(effect) => effect
                .outputs_mut()
                .into_iter()
                .map(|(name, arrow)| (Exit::Named(name), arrow))
                .collect(),
        }
    }

    /// The arrow leaving at `exit`, when the node has one there.
    pub fn exit_mut(&mut self, exit: Exit) -> Option<&mut Arrow> {
        self.exits_mut()
            .into_iter()
            .find(|(at, _)| *at == exit)
            .map(|(_, arrow)| arrow)
    }

    /// Whether a flow halts at the node until time passes or someone answers, the only way a
    /// loop may come back around.
    pub fn pauses(&self) -> bool {
        match self {
            Node::Wait { .. } | Node::Dialogue(_) => true,
            Node::Effect(effect) => effect.pauses(),
            _ => false,
        }
    }
}
