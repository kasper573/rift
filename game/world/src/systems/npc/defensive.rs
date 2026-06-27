use bevy_ecs::prelude::Entity;

use super::{Ai, Hunt};
use crate::core::math::Rng;
use crate::systems::combat::Attackers;

pub struct DefensiveAi;

inventory::submit! {
    &DefensiveAi as &dyn Ai
}

impl Ai for DefensiveAi {
    fn name(&self) -> &str {
        "defensive"
    }
    fn wanders(&self, _rng: &mut Rng) -> bool {
        true
    }
    fn target(&self, hunt: &Hunt) -> Option<Entity> {
        hunt.nearest(hunt.players, |player| {
            hunt.world
                .get::<Attackers>(hunt.id)
                .is_some_and(|attackers| attackers.ids.contains(&player))
        })
    }
}
