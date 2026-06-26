use bevy_ecs::prelude::Entity;

use super::{Ai, Hunt};
use crate::core::math::Rng;

/// Chases the nearest player on sight.
#[derive(Clone, Copy, Default)]
pub struct AggressiveAi;

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
