use bevy_ecs::prelude::Entity;

use super::{Ai, Hunt};
use crate::core::math::Rng;

pub struct Protective;

impl Ai for Protective {
    fn wanders(&self, _rng: &mut Rng) -> bool {
        true
    }
    fn target(&self, hunt: &Hunt) -> Option<Entity> {
        hunt.enemies_by_pack
            .get(&hunt.pack)
            .and_then(|enemies| hunt.nearest(enemies, |_| true))
    }
}
