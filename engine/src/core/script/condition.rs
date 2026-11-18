use std::sync::Arc;

use bevy_ecs::prelude::*;

use super::{Compiler, Eval, Expr, FlowCx, ScriptFault, Value, ValueType};
use crate::core::content::Content;

/// A condition compiled once to be asked of many.
#[derive(Clone)]
pub struct CompiledCondition {
    eval: Arc<Eval>,
    /// Whether it holds alike whoever it is asked about.
    shared: bool,
}

impl CompiledCondition {
    pub fn new(content: &Content, condition: &Expr) -> CompiledCondition {
        let mut reads_someone = unshared_read(condition).is_some();
        condition.visit(&mut |expr| {
            if let Expr::Query(query) = expr {
                reads_someone |= query.reads_subject();
            }
        });
        CompiledCondition {
            eval: Arc::new(compiled(content, condition, ValueType::Boolean)),
            shared: !reads_someone,
        }
    }

    pub fn holds(&self, world: &World, subject: Entity) -> bool {
        run(world, subject, &self.eval).truthy()
    }

    /// Whether it holds for everyone, when it holds alike whoever it is asked about.
    pub fn shared(&self, world: &World) -> Option<bool> {
        self.shared
            .then(|| (self.eval)(world, &FlowCx::nobody()).truthy())
    }
}

pub fn holds(world: &World, subject: Entity, condition: &Expr) -> bool {
    eval(world, subject, condition, ValueType::Boolean).truthy()
}

/// `expr` read as a value of the type `want` for `subject`.
pub fn eval(world: &World, subject: Entity, expr: &Expr, want: ValueType) -> Value {
    run(
        world,
        subject,
        &compiled(world.resource::<Content>(), expr, want),
    )
}

pub fn check(content: &Content, condition: &Expr) -> Result<(), ScriptFault> {
    Compiler::standalone(content)
        .expr(condition, ValueType::Boolean)
        .map(|_| ())
}

pub fn check_value(content: &Content, value: &Expr) -> Result<(), ScriptFault> {
    Compiler::standalone(content)
        .expr(value, ValueType::Text)
        .map(|_| ())
}

/// The parts a condition must all hold in: its `all`, or itself.
pub fn parts(condition: &Expr) -> Vec<&Expr> {
    match condition {
        Expr::All { of: parts } => parts.iter().collect(),
        other => vec![other],
    }
}

/// What `condition` reads that is not the same whoever asks, if anything: something of whom it
/// is asked about, or of the script asking.
pub fn unshared_read(condition: &Expr) -> Option<&'static str> {
    let mut unshared = None;
    condition.visit(&mut |expr| {
        let reads = match expr {
            Expr::Query(query) if !query.is_shared() => query.name(),
            Expr::Local { .. } | Expr::State { .. } | Expr::Event { .. } => expr.kind().name(),
            _ => return,
        };
        unshared.get_or_insert(reads);
    });
    unshared
}

fn run(world: &World, subject: Entity, compiled: &Eval) -> Value {
    compiled(world, &FlowCx::standalone(subject))
}

fn compiled(content: &Content, condition: &Expr, want: ValueType) -> Eval {
    Compiler::standalone(content)
        .expr(condition, want)
        .unwrap_or_else(|fault| {
            panic!("condition does not compile ({fault}); check_content should have refused it")
        })
}
