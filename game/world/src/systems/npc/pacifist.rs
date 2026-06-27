use bevy_ecs::prelude::Entity;

use super::{Ai, Hunt};
use crate::core::math::Rng;

const WANDER_CHANCE: f32 = 0.4;

pub struct Pacifist;

impl Ai for Pacifist {
    fn wanders(&self, rng: &mut Rng) -> bool {
        rng.rand_float() < WANDER_CHANCE
    }
    fn target(&self, _hunt: &Hunt) -> Option<Entity> {
        None
    }
}
