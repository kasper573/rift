use bevy_ecs::prelude::*;
use serde::Deserialize;

use super::Requirement;
use crate::core::table::Id;
use crate::systems::job::{self, JobDef};

/// Requires the player to be of a particular job.
#[derive(Deserialize, Clone, Debug)]
pub struct Job {
    #[serde(deserialize_with = "Id::<JobDef>::deserialize_named")]
    pub job: Id<JobDef>,
}

impl Requirement for Job {
    fn met(&self, world: &World, player: Entity) -> bool {
        world
            .get::<job::Job>(player)
            .is_some_and(|held| held.def == self.job)
    }
}
