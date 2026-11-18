use std::any::Any;

use super::kinds::registered_node;
use super::{Arrow, Compiler, EffectCall, EffectLook, Reach, RunnerKind, ScriptFault};

/// Something a script does to the game, as the mechanic it is about declares it.
pub trait EffectDef: dyn_clone::DynClone + Any + Send + Sync {
    /// The kinds of owner the effect is for; every kind when it names none.
    fn owners(&self) -> &'static [RunnerKind] {
        &[]
    }

    fn needs_subject(&self) -> bool;

    fn outputs(&self) -> Vec<(&'static str, &Arrow)>;

    fn outputs_mut(&mut self) -> Vec<(&'static str, &mut Arrow)>;

    fn pauses(&self) -> bool {
        false
    }

    fn keeps_box(&self) -> bool {
        false
    }

    fn compile<'c>(&'c self, cx: &mut Compiler<'c>) -> Result<EffectCall, ScriptFault>;

    /// A look telling, from the world alone, what the effect answers when it has nothing to do:
    /// a flow reads through such an effect as through any step that only reads, on every core.
    /// An effect that always acts has no look.
    fn look<'c>(&'c self, _cx: &mut Compiler<'c>) -> Result<Option<EffectLook>, ScriptFault> {
        Ok(None)
    }

    fn reach(&self, _cx: &Compiler) -> Vec<Reach> {
        Vec::new()
    }
}

dyn_clone::clone_trait_object!(EffectDef);

registered_node! {
    /// An effect any registered kind may be, as an effect node holds it.
    ScriptEffect of EffectDef, kinds effects, tag ByType
}
