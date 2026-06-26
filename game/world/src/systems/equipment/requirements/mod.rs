//! Equip gates: one per file implementing [`Requirement`], dispatched by [`RequirementKind`]
//! (`enum_dispatch`). An equipment item lists the gates it imposes; adding a gate kind is a new file
//! plus a variant — [`met`] never matches on a specific gate.

mod job;
mod level;
mod stat;

pub use job::Job;
pub use level::Level;
pub use stat::Stat;

use bevy_ecs::prelude::*;
use enum_dispatch::enum_dispatch;
use serde::Deserialize;

#[enum_dispatch]
pub trait Requirement {
    /// Whether `player` satisfies this gate.
    fn met(&self, world: &World, player: Entity) -> bool;
}

/// A gate on equipping an item. Stored in item defs (json only, never replicated), so the tagged
/// representation is fine here.
#[enum_dispatch(Requirement)]
#[derive(Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RequirementKind {
    Job(Job),
    Level(Level),
    Stat(Stat),
}

/// Whether `player` satisfies every gate.
pub fn met(world: &World, player: Entity, requirements: &[RequirementKind]) -> bool {
    requirements
        .iter()
        .all(|requirement| requirement.met(world, player))
}
