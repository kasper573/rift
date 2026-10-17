pub mod effects;
mod health;
pub mod widget;

pub use health::{
    apply_damage, current_health, dead, fraction, heal, is_dead, max_health, refill, regenerate,
    revive, settle_health,
};

use std::collections::BTreeMap;

use bevy_app::App;
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{IntoStaticStr, VariantArray};

#[derive(
    JsonSchema,
    Serialize,
    Deserialize,
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    VariantArray,
    IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "title_case")]
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
    pub const fn of(self, value: f32) -> Stat {
        Stat { kind: self, value }
    }

    pub const fn unit(self) -> Option<&'static str> {
        match self {
            StatKind::AttackDelay => Some("ms"),
            StatKind::MovementSpeed => Some("tiles/s"),
            StatKind::Range => Some("tiles"),
            StatKind::Health | StatKind::MaxHealth | StatKind::Damage | StatKind::AttackSpeed => {
                None
            }
        }
    }
}

#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Stat {
    #[serde(rename = "stat")]
    pub kind: StatKind,
    pub value: f32,
}

impl Stat {
    pub fn label(self) -> &'static str {
        self.kind.into()
    }
}

#[derive(JsonSchema, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(transparent)]
#[schemars(transform = schema_with_units)]
pub struct StatMap(pub BTreeMap<StatKind, f32>);

impl StatMap {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn get(&self, kind: StatKind) -> f32 {
        self.0.get(&kind).copied().unwrap_or(0.0)
    }

    pub fn listed(&self) -> impl Iterator<Item = Stat> + '_ {
        self.0.iter().map(|(&kind, &value)| kind.of(value))
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
}

/// Where an actor's stat modifiers come from besides its own `Stats`: its class levels, the
/// boosts its aspects hold, a chase under way.
pub type Source = fn(&World, Entity) -> Vec<Stat>;

#[derive(Resource, Default)]
pub struct Sources(Vec<Source>);

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.replicate::<Stats>().init_resource::<Sources>();
}

pub fn source(app: &mut App, source: Source) {
    app.world_mut().resource_mut::<Sources>().0.push(source);
}

pub fn modifiers(world: &World, entity: Entity) -> Vec<Stat> {
    world
        .resource::<Sources>()
        .0
        .iter()
        .flat_map(|source| source(world, entity))
        .collect()
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
    let delta: f32 = modifiers(world, entity)
        .into_iter()
        .filter(|stat| stat.kind == kind)
        .map(|stat| stat.value)
        .sum();
    base(world, entity, kind) + delta
}

pub fn effective_all(world: &World, entity: Entity) -> Stats {
    Stats(
        StatKind::VARIANTS
            .iter()
            .map(|&kind| kind.of(effective(world, entity, kind)))
            .collect(),
    )
}

fn schema_with_units(schema: &mut schemars::Schema) {
    let Some(properties) = schema.get_mut("properties").and_then(|v| v.as_object_mut()) else {
        return;
    };
    for &kind in StatKind::VARIANTS {
        let name = serde_json::to_value(kind)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        if let (Some(unit), Some(property)) = (
            kind.unit(),
            properties.get_mut(&name).and_then(|v| v.as_object_mut()),
        ) {
            property.insert(
                "x-unit".to_owned(),
                serde_json::Value::String(unit.to_owned()),
            );
        }
    }
}
