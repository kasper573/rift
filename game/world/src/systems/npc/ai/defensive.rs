use bevy_ecs::prelude::Entity;

use super::{Ai, Hunt};
use crate::core::math::Rng;
use crate::systems::combat::Attackers;

/// Peaceful until hit; then chases whoever has attacked it.
#[derive(Clone, Copy)]
pub struct Defensive;

impl Ai for Defensive {
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
