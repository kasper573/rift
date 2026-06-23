//! Combat: an entity's replicated [`Vitals`] and the attack request. The server systems that engage
//! targets, deal damage, and regenerate health live in the `server` crate.

use bevy_app::App;
use bevy_ecs::component::Component;
use bevy_ecs::entity::{Entity, MapEntities};
use bevy_ecs::message::Message;
use bevy_ecs::world::World;
use serde::{Deserialize, Serialize};

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;

    app.replicate::<Vitals>()
        .add_mapped_client_message::<AttackRequest>(Channel::Ordered);
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Vitals {
    pub health: f32,
    pub max: f32,
}

impl Vitals {
    pub fn heal(&mut self, amount: f32) {
        self.health = (self.health + amount).min(self.max);
    }

    pub fn damage(&mut self, amount: f32) {
        self.health = (self.health - amount).max(0.0);
    }

    pub fn refill(&mut self) {
        self.health = self.max;
    }

    pub fn fraction(&self) -> f32 {
        (self.health / self.max).clamp(0.0, 1.0)
    }

    pub fn is_dead(&self) -> bool {
        self.health <= 0.0
    }
}

#[derive(Message, Serialize, Deserialize, MapEntities, Clone, Debug, PartialEq)]
pub struct AttackRequest {
    #[entities]
    pub target: Entity,
}

pub fn is_dead(world: &World, entity: Entity) -> bool {
    world.get::<Vitals>(entity).is_some_and(Vitals::is_dead)
}
