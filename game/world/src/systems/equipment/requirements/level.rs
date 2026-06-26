use bevy_ecs::prelude::*;
use serde::Deserialize;

use super::Requirement;
use crate::systems::job;

/// Requires the player to have reached a job level.
#[derive(Deserialize, Clone, Debug)]
pub struct Level {
    pub level: u32,
}

impl Requirement for Level {
    fn met(&self, world: &World, player: Entity) -> bool {
        job::level(world, player) >= self.level
    }
}
