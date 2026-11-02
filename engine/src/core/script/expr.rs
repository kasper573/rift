use crate::core::content::SchemaMark;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{EnumDiscriminants, EnumString, IntoStaticStr, VariantArray};

use super::Vocabulary;

#[derive(JsonSchema, Clone, Serialize, Deserialize, EnumDiscriminants)]
#[serde(tag = "type", rename_all = "snake_case", bound = "")]
#[schemars(rename = "Expr", bound = "", transform = SchemaMark::Condition.with(true))]
#[strum_discriminants(
    name(ExprKind),
    derive(EnumString, IntoStaticStr, VariantArray),
    strum(serialize_all = "snake_case")
)]
pub enum Expr<V: Vocabulary> {
    Local {
        local: String,
    },
    State {
        state: String,
    },
    Event {
        event: String,
    },
    All {
        of: Vec<Expr<V>>,
    },
    Any {
        of: Vec<Expr<V>>,
    },
    Join {
        of: Vec<Expr<V>>,
    },
    Not {
        of: Box<Expr<V>>,
    },
    Round {
        of: Box<Expr<V>>,
    },
    Equal {
        a: Box<Expr<V>>,
        b: Box<Expr<V>>,
    },
    NotEqual {
        a: Box<Expr<V>>,
        b: Box<Expr<V>>,
    },
    Less {
        a: Box<Expr<V>>,
        b: Box<Expr<V>>,
    },
    AtMost {
        a: Box<Expr<V>>,
        b: Box<Expr<V>>,
    },
    More {
        a: Box<Expr<V>>,
        b: Box<Expr<V>>,
    },
    AtLeast {
        a: Box<Expr<V>>,
        b: Box<Expr<V>>,
    },
    Add {
        a: Box<Expr<V>>,
        b: Box<Expr<V>>,
    },
    Subtract {
        a: Box<Expr<V>>,
        b: Box<Expr<V>>,
    },
    Multiply {
        a: Box<Expr<V>>,
        b: Box<Expr<V>>,
    },
    Divide {
        a: Box<Expr<V>>,
        b: Box<Expr<V>>,
    },
    Min {
        a: Box<Expr<V>>,
        b: Box<Expr<V>>,
    },
    Max {
        a: Box<Expr<V>>,
        b: Box<Expr<V>>,
    },
    #[serde(untagged)]
    #[strum_discriminants(strum(serialize = "boolean"))]
    Bool(bool),
    #[serde(untagged)]
    #[strum_discriminants(strum(serialize = "integer"))]
    Int(i64),
    #[serde(untagged)]
    #[strum_discriminants(strum(serialize = "number"))]
    Num(f64),
    #[serde(untagged)]
    Text(String),
    #[serde(untagged)]
    Var {
        var: String,
    },
    #[serde(untagged)]
    Query(V::Query),
}

impl ExprKind {
    pub fn name(self) -> &'static str {
        self.into()
    }
}

pub const MAX_DEPTH: usize = 16;
pub const MAX_EXPR_NODES: usize = 64;
pub const MAX_LIST: usize = 32;

impl<V: Vocabulary> Expr<V> {
    pub fn kind(&self) -> ExprKind {
        self.into()
    }

    /// The comparison or arithmetic of `kind` between `a` and `b`, when `kind` is one.
    pub fn binary(kind: ExprKind, a: Box<Expr<V>>, b: Box<Expr<V>>) -> Option<Expr<V>> {
        Some(match kind {
            ExprKind::Equal => Expr::Equal { a, b },
            ExprKind::NotEqual => Expr::NotEqual { a, b },
            ExprKind::Less => Expr::Less { a, b },
            ExprKind::AtMost => Expr::AtMost { a, b },
            ExprKind::More => Expr::More { a, b },
            ExprKind::AtLeast => Expr::AtLeast { a, b },
            ExprKind::Add => Expr::Add { a, b },
            ExprKind::Subtract => Expr::Subtract { a, b },
            ExprKind::Multiply => Expr::Multiply { a, b },
            ExprKind::Divide => Expr::Divide { a, b },
            ExprKind::Min => Expr::Min { a, b },
            ExprKind::Max => Expr::Max { a, b },
            _ => return None,
        })
    }

    pub fn operands_mut(&mut self) -> Option<[&mut Box<Self>; 2]> {
        match self {
            Expr::Equal { a, b }
            | Expr::NotEqual { a, b }
            | Expr::Less { a, b }
            | Expr::AtMost { a, b }
            | Expr::More { a, b }
            | Expr::AtLeast { a, b }
            | Expr::Add { a, b }
            | Expr::Subtract { a, b }
            | Expr::Multiply { a, b }
            | Expr::Divide { a, b }
            | Expr::Min { a, b }
            | Expr::Max { a, b } => Some([a, b]),
            _ => None,
        }
    }

    pub fn children(&self) -> Vec<&Expr<V>> {
        match self {
            Expr::All { of } | Expr::Any { of } | Expr::Join { of } => of.iter().collect(),
            Expr::Not { of } | Expr::Round { of } => vec![of],
            Expr::Equal { a, b }
            | Expr::NotEqual { a, b }
            | Expr::Less { a, b }
            | Expr::AtMost { a, b }
            | Expr::More { a, b }
            | Expr::AtLeast { a, b }
            | Expr::Add { a, b }
            | Expr::Subtract { a, b }
            | Expr::Multiply { a, b }
            | Expr::Divide { a, b }
            | Expr::Min { a, b }
            | Expr::Max { a, b } => vec![a, b],
            _ => Vec::new(),
        }
    }

    pub fn vars(&self) -> Vec<&str> {
        let mut found = Vec::new();
        self.visit(&mut |expr| {
            if let Expr::Var { var } = expr {
                found.push(var.as_str());
            }
        });
        found
    }

    pub fn visit<'a>(&'a self, f: &mut impl FnMut(&'a Expr<V>)) {
        f(self);
        for child in self.children() {
            child.visit(f);
        }
    }

    pub fn depth(&self) -> usize {
        1 + self
            .children()
            .into_iter()
            .map(Expr::depth)
            .max()
            .unwrap_or(0)
    }

    pub fn size(&self) -> usize {
        1 + self.children().into_iter().map(Expr::size).sum::<usize>()
    }
}
