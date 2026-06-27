use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use super::{Scalar, Stat, base, effective};

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Health(pub f32);

#[derive(Clone, Copy)]
pub struct HealthStat;

inventory::submit! {
    &HealthStat as &dyn Stat
}

impl Scalar for HealthStat {
    type Component = Health;
    const NAME: &'static str = "Health";
    const LABEL: &'static str = "Health";
    fn read(health: &Health) -> f32 {
        health.0
    }
    fn make(value: f32) -> Health {
        Health(value)
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct MaxHealth(pub f32);

#[derive(Clone, Copy)]
pub struct MaxHealthStat;

inventory::submit! {
    &MaxHealthStat as &dyn Stat
}

impl Scalar for MaxHealthStat {
    type Component = MaxHealth;
    const NAME: &'static str = "MaxHealth";
    const LABEL: &'static str = "Max Health";
    fn read(max: &MaxHealth) -> f32 {
        max.0
    }
    fn make(value: f32) -> MaxHealth {
        MaxHealth(value)
    }
}

impl Health {
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
