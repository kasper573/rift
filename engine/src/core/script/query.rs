use std::any::Any;

use super::kinds::registered_node;
use super::{Compiler, Eval, ScriptFault, ValueType};

/// A question a script asks of the game, as the mechanic it is about declares it.
pub trait QueryDef: dyn_clone::DynClone + Any + Send + Sync {
    /// What the query yields, when its own kind says.
    fn static_type(&self) -> Option<ValueType>;

    fn value_type(&self, _cx: &Compiler) -> ValueType {
        self.static_type()
            .expect("a query without a type of its own tells it by its content")
    }

    /// Whether the answer depends on who the script's subject is, when none is named.
    fn reads_subject(&self) -> bool;

    /// Whether every world of the game answers the query alike: it reads no actor, and nothing
    /// a server knows that its clients don't.
    fn is_shared(&self) -> bool;

    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<Eval, ScriptFault>;
}

dyn_clone::clone_trait_object!(QueryDef);

registered_node! {
    /// A query any registered kind may be, as an expression holds it.
    ScriptQuery of QueryDef, kinds queries, tag ByType
}
