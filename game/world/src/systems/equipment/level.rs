use bevy_ecs::prelude::*;
use serde::Deserialize;

use super::Requirement;
use crate::systems::job;

#[derive(Deserialize, Clone, Debug)]
pub struct LevelRequirement {
    pub level: u32,
}

#[typetag::deserialize(name = "level")]
impl Requirement for LevelRequirement {
    fn met(&self, world: &World, player: Entity) -> bool {
        job::level(world, player) >= self.level
    }
}
