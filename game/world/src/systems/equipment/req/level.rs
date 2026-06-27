use bevy_ecs::prelude::*;
use serde::Deserialize;

use super::{Registered, Requirement, build};
use crate::systems::job;

#[derive(Deserialize, Clone, Debug)]
pub struct LevelRequirement(pub u32);

inventory::submit! {
    Registered { name: "level", build: build::<LevelRequirement> }
}

impl Requirement for LevelRequirement {
    fn met(&self, world: &World, player: Entity) -> bool {
        job::level(world, player) >= self.0
    }
}
