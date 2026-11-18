use std::any::Any;
use std::fmt::Debug;
use std::sync::Arc;

use super::ScriptFault;

/// Something a script may do to the game, as its mechanic declares it: the loader's checks and a
/// tool describing a script read what scripts may reach without running them.
pub trait ReachDef: Any + Debug + Send + Sync {
    /// What the reach lacks among `all` that its script reaches, when it only makes sense beside
    /// another.
    fn unmet(&self, _all: &[Reach]) -> Option<ScriptFault> {
        None
    }
}

/// A reach a script was found to have.
#[derive(Clone, Debug)]
pub struct Reach(Arc<dyn ReachDef>);

impl Reach {
    pub fn new<R: ReachDef>(reach: R) -> Reach {
        Reach(Arc::new(reach))
    }

    pub fn is<R: ReachDef>(&self) -> bool {
        self.downcast_ref::<R>().is_some()
    }

    pub fn downcast_ref<R: ReachDef>(&self) -> Option<&R> {
        let held: &dyn Any = &*self.0;
        held.downcast_ref()
    }

    pub fn unmet(&self, all: &[Reach]) -> Option<ScriptFault> {
        self.0.unmet(all)
    }
}
