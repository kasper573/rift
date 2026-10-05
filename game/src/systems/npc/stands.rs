use bevy_ecs::prelude::Entity;

use super::{Ai, Hunt};
use crate::core::math::Rng;

pub struct Stands;

impl Ai for Stands {
    fn wanders(&self, _rng: &mut Rng) -> bool {
        false
    }
    fn target(&self, _hunt: &Hunt) -> Option<Entity> {
        None
    }
}
