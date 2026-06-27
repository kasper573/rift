use bevy_ecs::prelude::Entity;

use super::{Ai, Hunt};
use crate::core::math::Rng;

/// Never attacks; wanders only sometimes, so a field of them isn't constantly milling.
const WANDER_CHANCE: f32 = 0.4;

pub struct PacifistAi;

inventory::submit! {
    &PacifistAi as &dyn Ai
}

impl Ai for PacifistAi {
    fn name(&self) -> &str {
        "pacifist"
    }
    fn wanders(&self, rng: &mut Rng) -> bool {
        rng.unit() < WANDER_CHANCE
    }
    fn target(&self, _hunt: &Hunt) -> Option<Entity> {
        None
    }
}
