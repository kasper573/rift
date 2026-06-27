mod health;

pub use health::{apply_damage, current_health, fraction, heal, is_dead, max_health, refill};

use std::mem::discriminant;

use bevy_app::App;
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::systems::effect::{self, EffectContext};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum Stat {
    Health(f32),
    MaxHealth(f32),
    Damage(f32),
    AttackSpeed(f32),
    AttackDelay(f32),
    MovementSpeed(f32),
    Range(f32),
}

impl Stat {
    /// The stat constructors, used both to enumerate stats and to address one by kind (e.g.
    /// `effective(world, entity, Stat::Damage)`).
    pub const ALL: [fn(f32) -> Stat; 7] = [
        Stat::Health,
        Stat::MaxHealth,
        Stat::Damage,
        Stat::AttackSpeed,
        Stat::AttackDelay,
        Stat::MovementSpeed,
        Stat::Range,
    ];

    pub fn value(self) -> f32 {
        let (Stat::Health(value)
        | Stat::MaxHealth(value)
        | Stat::Damage(value)
        | Stat::AttackSpeed(value)
        | Stat::AttackDelay(value)
        | Stat::MovementSpeed(value)
        | Stat::Range(value)) = self;
        value
    }

    pub fn label(self) -> &'static str {
        match self {
            Stat::Health(_) => "Health",
            Stat::MaxHealth(_) => "Max Health",
            Stat::Damage(_) => "Damage",
            Stat::AttackSpeed(_) => "Attack Speed",
            Stat::AttackDelay(_) => "Attack Delay",
            Stat::MovementSpeed(_) => "Move Speed",
            Stat::Range(_) => "Range",
        }
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Stats(pub Vec<Stat>);

impl Stats {
    pub fn get(&self, kind: fn(f32) -> Stat) -> f32 {
        let want = discriminant(&kind(0.0));
        self.0
            .iter()
            .find(|stat| discriminant(*stat) == want)
            .map_or(0.0, |stat| stat.value())
    }

    pub fn apply(&self, world: &mut World, entity: Entity) {
        world.entity_mut(entity).insert(self.clone());
    }
}

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.replicate::<Stats>();
}

pub fn snapshot(world: &World, entity: Entity) -> Stats {
    world.get::<Stats>(entity).cloned().unwrap_or_default()
}

pub fn base(world: &World, entity: Entity, kind: fn(f32) -> Stat) -> f32 {
    world
        .get::<Stats>(entity)
        .map_or(0.0, |stats| stats.get(kind))
}

pub fn effective(world: &World, entity: Entity, kind: fn(f32) -> Stat) -> f32 {
    let want = discriminant(&kind(0.0));
    let ctx = EffectContext {
        world,
        source: entity,
        target: entity,
    };
    let delta: f32 = effect::active_effects(world, entity)
        .iter()
        .flat_map(|effect| effect.compute(&ctx))
        .filter(|stat| discriminant(stat) == want)
        .map(|stat| stat.value())
        .sum();
    base(world, entity, kind) + delta
}

pub fn effective_all(world: &World, entity: Entity) -> Stats {
    Stats(
        Stat::ALL
            .iter()
            .map(|&kind| kind(effective(world, entity, kind)))
            .collect(),
    )
}
