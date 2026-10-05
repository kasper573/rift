use bevy_ecs::prelude::Entity;

use super::{Ai, Hunt};
use crate::core::math::Rng;
use crate::core::tiling::Tiles;

const WANDER_CHANCE: f32 = 0.01;
const LEASH: Tiles = Tiles(3.0);

pub struct Strolls;

impl Ai for Strolls {
    fn wanders(&self, rng: &mut Rng) -> bool {
        rng.rand_float() < WANDER_CHANCE
    }
    fn target(&self, _hunt: &Hunt) -> Option<Entity> {
        None
    }
    fn leash(&self) -> Option<Tiles> {
        Some(LEASH)
    }
}
