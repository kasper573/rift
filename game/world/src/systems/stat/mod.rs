//! Stats: the single representation of an actor's combat numbers. Each stat is a per-file ECS
//! component dispatched by [`StatKind`] (`enum_dispatch`); there is no lumped base/effective struct.
//! A *scalar* stat stores its base in a component; a *computed* stat derives its base from the entity.
//! The effective value combat reads is the base plus every active effect's delta for that stat —
//! summed here, on read, so effects stay immutable sets the stats system owns. Adding a stat is a new
//! file plus a [`StatKind`] variant; the registry iterates itself.

mod etc;
mod health;

pub use etc::*;
pub use health::*;

use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_ecs::world::EntityRef;
use enum_dispatch::enum_dispatch;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use strum::{EnumIter, IntoEnumIterator};

use crate::systems::effect::{self, EffectContext};

pub fn register(app: &mut App) {
    for stat in StatKind::all() {
        stat.replicate(app);
    }
}

/// One stat. Scalar stats back [`base`](Stat::base) with a component and [`set`](Stat::set) it;
/// computed stats derive `base` from the entity and ignore `set`/`replicate`.
#[enum_dispatch]
pub trait Stat {
    /// Id equal to the component name; used in tables and effect args.
    fn name(&self) -> &str;
    fn label(&self) -> &str;
    fn computed(&self) -> bool;
    /// The intrinsic value before effects.
    fn base(&self, entity: EntityRef) -> f32;
    /// Writes a scalar stat's base.
    fn set(&self, world: &mut World, entity: Entity, value: f32);
    /// Registers the stat's component for replication.
    fn replicate(&self, app: &mut App);
}

/// Every stat. `enum_dispatch` forwards [`Stat`] to the variant; `EnumIter` lets the registry iterate
/// itself, so adding a stat is just a new file plus a variant here.
#[enum_dispatch(Stat)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, EnumIter)]
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
    /// Every stat, so callers iterate the set without importing the iteration trait.
    pub fn all() -> impl Iterator<Item = StatKind> {
        StatKind::iter()
    }

    fn by_name(name: &str) -> Option<StatKind> {
        StatKind::all().find(|stat| stat.name() == name)
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
        let map = std::collections::HashMap::<StatKind, f32>::deserialize(deserializer)?;
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
            .map(|stat| (stat, base(world, entity, stat)))
            .collect(),
    );
    for command in effect::active_effects(world, entity) {
        set.merge(command.compute(&ctx));
    }
    set
}

/// Generates a scalar stat: a value component, a unit marker, and its [`Stat`] impl (`base`/`set` use
/// the component, `replicate` registers it). The stat's id is the component name. See the per-stat
/// files in this folder.
macro_rules! scalar_stat {
    ($component:ident, $marker:ident, $label:literal) => {
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

        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
        pub struct $marker;

        impl $crate::systems::stat::Stat for $marker {
            fn name(&self) -> &str {
                stringify!($component)
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
            fn replicate(&self, app: &mut bevy_app::App) {
                use bevy_replicon::prelude::*;
                app.replicate::<$component>();
            }
        }
    };
}
pub(crate) use scalar_stat;
