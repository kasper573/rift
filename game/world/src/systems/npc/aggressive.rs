use bevy_ecs::prelude::Entity;

use super::{Ai, Hunt};
use crate::core::math::Rng;

pub struct AggressiveAi;

inventory::submit! {
    &AggressiveAi as &dyn Ai
}

impl Ai for AggressiveAi {
    fn name(&self) -> &str {
        "aggressive"
    }
    fn wanders(&self, _rng: &mut Rng) -> bool {
        true
    }
    fn target(&self, hunt: &Hunt) -> Option<Entity> {
        hunt.nearest(hunt.players, |_| true)
    }
}
