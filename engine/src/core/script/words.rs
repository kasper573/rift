use std::any::Any;

use super::kinds::registered_slot;
use super::{Expr, ScriptFault};
use crate::core::content::Content;

/// What a dialogue node says or labels a choice with, holding the script values the words read.
pub trait WordsDef: dyn_clone::DynClone + Any + Send + Sync {
    fn values(&self) -> Vec<&Expr>;

    /// The rules the words keep, beyond what the schema says.
    fn check(&self, _content: &Content) -> Result<(), ScriptFault> {
        Ok(())
    }
}

dyn_clone::clone_trait_object!(WordsDef);

registered_slot! {
    /// A line of a dialogue node, as the one type registered for lines holds it.
    DialogueLine of WordsDef, slot lines
}

registered_slot! {
    /// The label of a dialogue choice, as the one type registered for labels holds it.
    ChoiceLabel of WordsDef, slot labels
}

/// Where in the game a script sends someone: the one type registered for places.
pub trait PlaceDef: dyn_clone::DynClone + Any + Send + Sync {}

dyn_clone::clone_trait_object!(PlaceDef);

registered_slot! {
    /// A place a script variable is given, as the one type registered for places holds it.
    PlaceValue of PlaceDef, slot places
}
