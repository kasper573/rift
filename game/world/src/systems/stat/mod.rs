//! Stats: the single representation of an actor's combat numbers. Each stat is a per-file ECS
//! component behind a [`Stat`] marker submitted to an inventory and addressed by its [`StatId`];
//! there is no lumped base/effective struct. A stat stores its base in a component (see [`Scalar`]);
//! the effective value combat reads is the base plus every active effect's delta for that stat —
//! summed here, on read, so effects stay immutable sets the stats system owns. Adding a stat is a new
//! file plus its submission; the registry iterates the submissions.

mod attack_delay;
mod attack_speed;
mod damage;
mod health;
mod movement_speed;
mod range;

pub use attack_delay::*;
pub use attack_speed::*;
pub use damage::*;
pub use health::*;
pub use movement_speed::*;
pub use range::*;

use bevy_app::App;
use bevy_ecs::component::Mutable;
use bevy_ecs::prelude::*;
use bevy_ecs::world::EntityRef;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::systems::effect::{self, EffectContext};

inventory::collect!(&'static dyn Stat);

pub fn register(app: &mut App) {
    // Replicate stats name-sorted, so the native server and the wasm client assign each stat component
    // the same replication id despite their differing inventory link order.
    let mut stats: Vec<&'static dyn Stat> =
        inventory::iter::<&'static dyn Stat>().copied().collect();
    stats.sort_by_key(|stat| stat.name());
    for stat in stats {
        stat.replicate(app);
    }
}

/// One stat: how its base value is named, labelled, read, written, and replicated. The blanket impl
/// over [`Scalar`] provides this for every stat, so a new one is just a marker type.
pub trait Stat: Send + Sync {
    /// Id equal to the component name; used in tables and effect args.
    fn name(&self) -> &str;
    fn label(&self) -> &str;
    /// The intrinsic value before effects.
    fn base(&self, entity: EntityRef) -> f32;
    /// Writes the stat's base.
    fn set(&self, world: &mut World, entity: Entity, value: f32);
    /// Registers the stat's component for replication.
    fn replicate(&self, app: &mut App);
}

/// A stat's identity: the name its marker reports — how it is written in tables and effect args, and
/// what [`def`] resolves back to a [`Stat`]. Every stat marker converts into one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StatId(&'static str);

impl StatId {
    pub fn name(self) -> &'static str {
        self.0
    }
    pub fn label(self) -> &'static str {
        def(self).label()
    }
}

impl<S: Scalar> From<S> for StatId {
    fn from(_: S) -> StatId {
        StatId(S::NAME)
    }
}

pub fn all() -> impl Iterator<Item = StatId> {
    inventory::iter::<&'static dyn Stat>()
        .copied()
        .map(|stat| StatId(stat.name()))
}

/// Every [`StatId`] is a registered stat's name, so the lookup always hits.
fn def(id: StatId) -> &'static dyn Stat {
    inventory::iter::<&'static dyn Stat>()
        .copied()
        .find(|stat| stat.name() == id.0)
        .expect("a registered stat")
}

impl Serialize for StatId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for StatId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        all()
            .find(|stat| stat.0 == name)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown stat '{name}'")))
    }
}

/// A set of stat values: an authored base (every scalar stat) or a delta an effect contributes. The
/// stats system sums these; effects just return them, so an effect's contribution drops out cleanly
/// once it is no longer active.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StatSet(Vec<(StatId, f32)>);

impl StatSet {
    pub fn single(stat: StatId, amount: f32) -> StatSet {
        StatSet(vec![(stat, amount)])
    }

    pub fn get(&self, stat: StatId) -> f32 {
        self.0
            .iter()
            .filter(|(s, _)| *s == stat)
            .map(|(_, amount)| amount)
            .sum()
    }

    pub fn add(&mut self, stat: StatId, amount: f32) {
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

    /// Writes each stat's base onto `entity`.
    pub fn apply(&self, world: &mut World, entity: Entity) {
        for &(stat, value) in &self.0 {
            def(stat).set(world, entity, value);
        }
    }

    /// Every stat's current base on `entity` — the actor's whole stat state, e.g. to carry it across a
    /// portal and re-`apply` it on arrival.
    pub fn snapshot(world: &World, entity: Entity) -> StatSet {
        world.get_entity(entity).map_or_else(
            |_| StatSet::default(),
            |entity| StatSet(all().map(|stat| (stat, def(stat).base(entity))).collect()),
        )
    }
}

impl<'de> Deserialize<'de> for StatSet {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let map = std::collections::HashMap::<StatId, f32>::deserialize(deserializer)?;
        for stat in all() {
            if !map.contains_key(&stat) {
                return Err(Error::custom(format!("missing stat '{}'", stat.name())));
            }
        }
        Ok(StatSet(map.into_iter().collect()))
    }
}

/// The intrinsic value of `stat` on `entity`, before effects.
pub fn base(world: &World, entity: Entity, stat: StatId) -> f32 {
    world
        .get_entity(entity)
        .map_or(0.0, |entity| def(stat).base(entity))
}

/// `base` plus every active effect's delta for `stat`. The same result on server and client, since
/// both run it over the same components and replicated effect sources.
pub fn effective(world: &World, entity: Entity, stat: StatId) -> f32 {
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
        all()
            .map(|stat| (stat, base(world, entity, stat)))
            .collect(),
    );
    for command in effect::active_effects(world, entity) {
        set.merge(command.compute(&ctx));
    }
    set
}

/// A scalar stat: a marker type naming its storage [`Component`](Scalar::Component) and its display
/// label. The blanket [`Stat`] impl below turns any `Scalar` into a full stat — reading the base from
/// its component, writing it, and registering it for replication — so the dispatch logic lives once
/// here, and a new scalar stat is just a component newtype, a marker, and a small `Scalar` impl (see
/// the per-stat files in this folder).
pub trait Scalar: Copy + Send + Sync + 'static {
    /// The component holding this stat's base value on an entity.
    type Component: Component<Mutability = Mutable> + Clone + Serialize + DeserializeOwned;
    /// Id equal to the component name; used in tables and effect args.
    const NAME: &'static str;
    const LABEL: &'static str;
    fn read(component: &Self::Component) -> f32;
    fn make(value: f32) -> Self::Component;
}

impl<S: Scalar> Stat for S {
    fn name(&self) -> &str {
        S::NAME
    }
    fn label(&self) -> &str {
        S::LABEL
    }
    fn base(&self, entity: EntityRef) -> f32 {
        entity.get::<S::Component>().map_or(0.0, S::read)
    }
    fn set(&self, world: &mut World, entity: Entity, value: f32) {
        world.entity_mut(entity).insert(S::make(value));
    }
    fn replicate(&self, app: &mut App) {
        use bevy_replicon::prelude::*;
        app.replicate::<S::Component>();
    }
}
