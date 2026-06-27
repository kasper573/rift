use bevy_ecs::prelude::Entity;

use super::{Ai, Hunt};
use crate::core::math::Rng;

pub struct ProtectiveAi;

inventory::submit! {
    &ProtectiveAi as &dyn Ai
}

impl Ai for ProtectiveAi {
    fn name(&self) -> &str {
        "protective"
    }
    fn wanders(&self, _rng: &mut Rng) -> bool {
        true
    }
    fn target(&self, hunt: &Hunt) -> Option<Entity> {
        hunt.by_group
            .get(&hunt.group)
            .and_then(|enemies| hunt.nearest(enemies, |_| true))
    }
}
