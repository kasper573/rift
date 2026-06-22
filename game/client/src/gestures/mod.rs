mod attack;
mod grab;
mod walk;

use bevy::prelude::*;

/// One way the primary button can be used, owned by the system that handles it. The input layer tries
/// the active intents in order and the first to claim a press owns it until release, so two never run at
/// once; a press no intent claims is left untouched. A new interaction is a new file here plus a line in
/// `all`.
pub trait InputIntent: Send + Sync {
    /// Does this claim the press starting now? Reads the world — the cursor and what is under it.
    fn claims(&self, world: &mut World) -> bool;
    /// Drive the claimed press: `start` is true on the down-frame, false on each held frame after.
    fn drive(&self, world: &mut World, start: bool);
}

/// Every active intent, most specific first; the first to claim a press owns it.
pub const ALL: &[&dyn InputIntent] = &[&grab::Grab, &attack::Attack, &walk::Walk];

pub(crate) fn init(app: &mut App) {
    app.init_resource::<walk::HeldMove>();
}
