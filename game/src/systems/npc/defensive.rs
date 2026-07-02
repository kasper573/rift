use bevy_ecs::prelude::Entity;

use super::{Ai, Hunt};
use crate::core::math::Rng;
use crate::systems::combat::Attackers;

pub struct Defensive;

impl Ai for Defensive {
    fn wanders(&self, _rng: &mut Rng) -> bool {
        true
    }
    fn target(&self, hunt: &Hunt) -> Option<Entity> {
        let attackers = hunt.world.get::<Attackers>(hunt.id)?;
        hunt.nearest(hunt.players, |player| attackers.ids.contains(&player))
    }
}
