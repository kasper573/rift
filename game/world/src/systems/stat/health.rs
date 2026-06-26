//! Health is a current resource (mutated directly by combat) and max_health its cap; by convention
//! they pair, and the helpers that read/spend health live here with them.

use bevy_ecs::prelude::*;

use super::{base, effective, scalar_stat};

scalar_stat!(Health, HealthStat, "Health");
scalar_stat!(MaxHealth, MaxHealthStat, "Max Health");

impl Health {
    /// An empty pool is death — the one definition of it, so callers with a `&Health` in hand (a
    /// query) and those with only an entity ([`is_dead`]) agree.
    pub fn depleted(&self) -> bool {
        self.0 <= 0.0
    }
}

pub fn current_health(world: &World, entity: Entity) -> f32 {
    base(world, entity, HealthStat.into())
}

pub fn max_health(world: &World, entity: Entity) -> f32 {
    effective(world, entity, MaxHealthStat.into())
}

pub fn is_dead(world: &World, entity: Entity) -> bool {
    world.get::<Health>(entity).is_some_and(Health::depleted)
}

pub fn fraction(world: &World, entity: Entity) -> f32 {
    let max = max_health(world, entity);
    if max <= 0.0 {
        0.0
    } else {
        (current_health(world, entity) / max).clamp(0.0, 1.0)
    }
}

pub fn apply_damage(world: &mut World, entity: Entity, amount: f32) {
    if let Some(mut health) = world.get_mut::<Health>(entity) {
        health.0 = (health.0 - amount).max(0.0);
    }
}

pub fn heal(world: &mut World, entity: Entity, amount: f32) {
    let max = max_health(world, entity);
    if let Some(mut health) = world.get_mut::<Health>(entity) {
        health.0 = (health.0 + amount).min(max);
    }
}

pub fn refill(world: &mut World, entity: Entity) {
    let max = max_health(world, entity);
    if let Some(mut health) = world.get_mut::<Health>(entity) {
        health.0 = max;
    }
}
