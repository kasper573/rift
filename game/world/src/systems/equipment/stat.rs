use bevy_ecs::prelude::*;
use serde::Deserialize;

use super::Requirement;
use crate::systems::stat::{self, StatId};

#[derive(Deserialize, Clone, Debug)]
pub struct StatRequirement {
    pub stat: StatId,
    pub min: f32,
}

#[typetag::deserialize(name = "stat")]
impl Requirement for StatRequirement {
    fn met(&self, world: &World, player: Entity) -> bool {
        stat::effective(world, player, self.stat) >= self.min
    }
}
