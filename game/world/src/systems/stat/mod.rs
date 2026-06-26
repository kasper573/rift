//! Stats: the single representation of an actor's combat numbers. Each stat is a per-file ECS
//! component (see [`definitions`]) dispatched by [`StatKind`] (`enum_dispatch`); there is no lumped
//! base/effective struct. A *scalar* stat stores its base in a component; a *computed* stat derives
//! its base from the entity. The effective value combat reads is the base plus every active effect's
//! delta for that stat — summed here, on read, so effects stay immutable sets the stats system owns.

pub mod definitions;

use std::collections::HashMap;

use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_ecs::world::EntityRef;
use enum_dispatch::enum_dispatch;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::systems::effect::{self, EffectContext};

pub use definitions::{
    AttackDelay, AttackDelayStat, AttackSpeed, AttackSpeedStat, Damage, DamageStat, Health,
    HealthStat, MaxHealth, MaxHealthStat, MovementSpeed, MovementSpeedStat, Range, RangeStat,
};

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.replicate::<Health>()
        .replicate::<MaxHealth>()
        .replicate::<Damage>()
        .replicate::<AttackSpeed>()
        .replicate::<AttackDelay>()
        .replicate::<Range>()
        .replicate::<MovementSpeed>();
}

/// One stat. Scalar stats back [`base`](Stat::base) with a component and [`set`](Stat::set) it;
/// computed stats derive `base` from the entity and ignore `set`.
#[enum_dispatch]
pub trait Stat {
    /// Id equal to the snake_case name used in tables and effect args.
    fn name(&self) -> &str;
    fn label(&self) -> &str;
    fn computed(&self) -> bool;
    /// The intrinsic value before effects.
    fn base(&self, entity: EntityRef) -> f32;
    /// Writes a scalar stat's base; a no-op for computed stats.
    fn set(&self, world: &mut World, entity: Entity, value: f32);
}

/// Every stat. `enum_dispatch` forwards [`Stat`] to the variant; a new stat is a new file plus a
/// variant here (and a `replicate` line above for a scalar one).
#[enum_dispatch(Stat)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StatKind {
    Health(HealthStat),
    MaxHealth(MaxHealthStat),
    Damage(DamageStat),
    AttackSpeed(AttackSpeedStat),
    AttackDelay(AttackDelayStat),
    Range(RangeStat),
    MovementSpeed(MovementSpeedStat),
}

impl StatKind {
    pub fn all() -> [StatKind; 7] {
        [
            HealthStat.into(),
            MaxHealthStat.into(),
            DamageStat.into(),
            AttackSpeedStat.into(),
            AttackDelayStat.into(),
            RangeStat.into(),
            MovementSpeedStat.into(),
        ]
    }

    fn by_name(name: &str) -> Option<StatKind> {
        StatKind::all().into_iter().find(|stat| stat.name() == name)
    }
}

impl Serialize for StatKind {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.name().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for StatKind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        StatKind::by_name(&name)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown stat '{name}'")))
    }
}

// --- StatSet: an immutable set of stat values, summed by the stats system ---

/// A set of stat values: an authored base (every scalar stat) or a delta an effect contributes. The
/// stats system sums these; effects just return them, so an effect's contribution drops out cleanly
/// once it is no longer active.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StatSet(Vec<(StatKind, f32)>);

impl StatSet {
    pub fn single(stat: StatKind, amount: f32) -> StatSet {
        StatSet(vec![(stat, amount)])
    }

    pub fn get(&self, stat: StatKind) -> f32 {
        self.0
            .iter()
            .filter(|(s, _)| *s == stat)
            .map(|(_, amount)| amount)
            .sum()
    }

    pub fn add(&mut self, stat: StatKind, amount: f32) {
        match self.0.iter_mut().find(|(s, _)| *s == stat) {
            Some((_, current)) => *current += amount,
            None => self.0.push((stat, amount)),
        }
    }

    pub fn merge(&mut self, other: StatSet) {
        for (stat, amount) in other.0 {
            self.add(stat, amount);
        }
    }

    pub fn describe(&self) -> String {
        self.0
            .iter()
            .map(|(stat, amount)| format!("{amount:+} {}", stat.label()))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Writes each scalar stat onto `entity` (computed stats derive themselves and are skipped).
    pub fn apply(&self, world: &mut World, entity: Entity) {
        for &(stat, value) in &self.0 {
            if !stat.computed() {
                stat.set(world, entity, value);
            }
        }
    }

    /// Every scalar stat's current base on `entity` — the actor's whole stat state, e.g. to carry it
    /// across a portal and re-`apply` it on arrival.
    pub fn snapshot(world: &World, entity: Entity) -> StatSet {
        world.get_entity(entity).map_or_else(
            |_| StatSet::default(),
            |entity| {
                StatSet(
                    StatKind::all()
                        .into_iter()
                        .filter(|stat| !stat.computed())
                        .map(|stat| (stat, stat.base(entity)))
                        .collect(),
                )
            },
        )
    }
}

impl<'de> Deserialize<'de> for StatSet {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let map = HashMap::<StatKind, f32>::deserialize(deserializer)?;
        for stat in StatKind::all() {
            if stat.computed() {
                if map.contains_key(&stat) {
                    return Err(Error::custom(format!(
                        "computed stat '{}' cannot be authored",
                        stat.name()
                    )));
                }
            } else if !map.contains_key(&stat) {
                return Err(Error::custom(format!("missing stat '{}'", stat.name())));
            }
        }
        Ok(StatSet(map.into_iter().collect()))
    }
}

// --- reading effective stats (base + active effect deltas) ---

/// The intrinsic value of `stat` on `entity`, before effects.
pub fn base(world: &World, entity: Entity, stat: StatKind) -> f32 {
    world
        .get_entity(entity)
        .map_or(0.0, |entity| stat.base(entity))
}

/// `base` plus every active effect's delta for `stat`. The same result on server and client, since
/// both run it over the same components and replicated effect sources.
pub fn effective(world: &World, entity: Entity, stat: StatKind) -> f32 {
    let ctx = EffectContext {
        world,
        source: entity,
        target: entity,
    };
    let delta: f32 = effect::active_effects(world, entity)
        .iter()
        .map(|command| command.compute(&ctx).get(stat))
        .sum();
    base(world, entity, stat) + delta
}

/// Every stat's effective value at once (effects summed a single time) — for callers that read many.
pub fn effective_all(world: &World, entity: Entity) -> StatSet {
    let ctx = EffectContext {
        world,
        source: entity,
        target: entity,
    };
    let mut set = StatSet(
        StatKind::all()
            .into_iter()
            .map(|stat| (stat, base(world, entity, stat)))
            .collect(),
    );
    for command in effect::active_effects(world, entity) {
        set.merge(command.compute(&ctx));
    }
    set
}

// --- health: a stat by convention paired with max_health (current HP, mutated directly) ---

pub fn current_health(world: &World, entity: Entity) -> f32 {
    base(world, entity, HealthStat.into())
}

pub fn max_health(world: &World, entity: Entity) -> f32 {
    effective(world, entity, MaxHealthStat.into())
}

pub fn is_dead(world: &World, entity: Entity) -> bool {
    world.get::<Health>(entity).is_some_and(|h| h.0 <= 0.0)
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

// --- codegen for the scalar stats (component + marker + Stat impl) ---

macro_rules! scalar_stat {
    ($component:ident, $marker:ident, $name:literal, $label:literal) => {
        #[derive(
            bevy_ecs::component::Component,
            Clone,
            Copy,
            Debug,
            PartialEq,
            serde::Serialize,
            serde::Deserialize,
        )]
        pub struct $component(pub f32);

        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub struct $marker;

        impl $crate::systems::stat::Stat for $marker {
            fn name(&self) -> &str {
                $name
            }
            fn label(&self) -> &str {
                $label
            }
            fn computed(&self) -> bool {
                false
            }
            fn base(&self, entity: bevy_ecs::world::EntityRef) -> f32 {
                entity.get::<$component>().map_or(0.0, |stat| stat.0)
            }
            fn set(
                &self,
                world: &mut bevy_ecs::world::World,
                entity: bevy_ecs::prelude::Entity,
                value: f32,
            ) {
                world.entity_mut(entity).insert($component(value));
            }
        }
    };
}
pub(crate) use scalar_stat;
