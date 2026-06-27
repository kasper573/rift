mod health;

pub use health::{apply_damage, current_health, fraction, heal, is_dead, max_health, refill};

use bevy_app::App;
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};
use strum::{IntoStaticStr, VariantArray};

use crate::systems::effect::{self, EffectContext};

#[derive(
    Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, VariantArray, IntoStaticStr,
)]
pub enum StatKind {
    Health,
    MaxHealth,
    Damage,
    AttackSpeed,
    AttackDelay,
    MovementSpeed,
    Range,
}

impl StatKind {
    #[allow(clippy::new_ret_no_self)]
    pub const fn new(self, value: f32) -> Stat {
        Stat { kind: self, value }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Stat {
    pub kind: StatKind,
    pub value: f32,
}

impl Stat {
    pub fn label(self) -> &'static str {
        self.kind.into()
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Stats(pub Vec<Stat>);

impl Stats {
    pub fn get(&self, kind: StatKind) -> f32 {
        self.0
            .iter()
            .find(|stat| stat.kind == kind)
            .map_or(0.0, |stat| stat.value)
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

pub fn base(world: &World, entity: Entity, kind: StatKind) -> f32 {
    world
        .get::<Stats>(entity)
        .map_or(0.0, |stats| stats.get(kind))
}

pub fn effective(world: &World, entity: Entity, kind: StatKind) -> f32 {
    let ctx = EffectContext {
        world,
        source: entity,
        target: entity,
    };
    let delta: f32 = effect::active_effects(world, entity)
        .iter()
        .flat_map(|effect| effect.compute(&ctx))
        .filter(|stat| stat.kind == kind)
        .map(|stat| stat.value)
        .sum();
    base(world, entity, kind) + delta
}

pub fn effective_all(world: &World, entity: Entity) -> Stats {
    Stats(
        StatKind::VARIANTS
            .iter()
            .map(|&kind| kind.new(effective(world, entity, kind)))
            .collect(),
    )
}
