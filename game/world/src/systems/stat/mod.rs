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

pub trait Stat: Send + Sync {
    fn name(&self) -> &str;
    fn label(&self) -> &str;
    fn base(&self, entity: EntityRef) -> f32;
    fn set(&self, world: &mut World, entity: Entity, value: f32);
    fn replicate(&self, app: &mut App);
}

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

    pub fn apply(&self, world: &mut World, entity: Entity) {
        for &(stat, value) in &self.0 {
            def(stat).set(world, entity, value);
        }
    }

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

pub fn base(world: &World, entity: Entity, stat: StatId) -> f32 {
    world
        .get_entity(entity)
        .map_or(0.0, |entity| def(stat).base(entity))
}

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

pub trait Scalar: Copy + Send + Sync + 'static {
    type Component: Component<Mutability = Mutable> + Clone + Serialize + DeserializeOwned;
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
