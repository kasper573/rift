use bevy_ecs::prelude::*;

use super::{StatKind, Stats, base, effective};

pub fn current_health(world: &World, entity: Entity) -> f32 {
    base(world, entity, StatKind::Health)
}

pub fn max_health(world: &World, entity: Entity) -> f32 {
    effective(world, entity, StatKind::MaxHealth)
}

pub fn is_dead(world: &World, entity: Entity) -> bool {
    world
        .get::<Stats>(entity)
        .is_some_and(|stats| stats.get(StatKind::Health) <= 0.0)
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
    set_health(world, entity, |health, _| (health - amount).max(0.0));
}

pub fn heal(world: &mut World, entity: Entity, amount: f32) {
    set_health(world, entity, |health, max| (health + amount).min(max));
}

pub fn refill(world: &mut World, entity: Entity) {
    set_health(world, entity, |_, max| max);
}

fn set_health(world: &mut World, entity: Entity, f: impl Fn(f32, f32) -> f32) {
    let max = max_health(world, entity);
    if let Some(mut stats) = world.get_mut::<Stats>(entity) {
        for stat in stats.0.iter_mut() {
            if stat.kind == StatKind::Health {
                stat.value = f(stat.value, max);
            }
        }
    }
}
