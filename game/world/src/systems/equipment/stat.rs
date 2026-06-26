use bevy_ecs::prelude::*;
use serde::Deserialize;

use super::Requirement;
use crate::systems::stat::{self, StatKind};

/// Requires one of the player's effective stats to be at least `min`.
#[derive(Deserialize, Clone, Debug)]
pub struct StatRequirement {
    pub stat: StatKind,
    pub min: f32,
}

impl Requirement for StatRequirement {
    fn met(&self, world: &World, player: Entity) -> bool {
        stat::effective(world, player, self.stat) >= self.min
    }
}
