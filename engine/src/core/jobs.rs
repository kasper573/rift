//! Work handed off the main thread and taken back. Live, a frame takes each piece once it is done
//! and never waits on one; a world stepped frame by frame for a picture takes each the frame after
//! it was handed out, so it comes out alike every time.

use bevy::prelude::*;
use bevy::tasks::{Task, block_on};

/// How the world takes back the work it hands off the main thread.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Handoff {
    /// Each piece once it is done, so no frame waits on one.
    #[default]
    AsDone,
    /// Every piece the frame after it was handed out, waited on if need be.
    NextFrame,
}

impl Handoff {
    /// How `world` takes back its work: as its [`Handoff`] says, as it is done by default.
    pub fn of(world: &World) -> Handoff {
        world.get_resource::<Handoff>().copied().unwrap_or_default()
    }

    /// What `task` came to, if it is to be taken now; else the task, still at work.
    pub fn take<T>(self, task: Task<T>) -> Result<T, Task<T>> {
        match self {
            Handoff::AsDone if !task.is_finished() => Err(task),
            Handoff::AsDone | Handoff::NextFrame => Ok(block_on(task)),
        }
    }
}
