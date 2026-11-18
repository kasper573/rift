pub mod query;
mod vital;
pub mod widget;

pub use vital::{
    Immortal, SettleVitals, VitalVfx, apply_damage, check_vfx, current, current_health, dead,
    drain, heal, is_dead, max_health, most, refill, restore, revive, stands,
};

use crate::core::content::SchemaMark;
use crate::core::script::Vocabulary;
use std::collections::BTreeMap;

use crate::systems::messages::messages;
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
    Mana,
    MaxMana,
    Damage,
    AttackSpeed,
    AttackDelay,
    MovementSpeed,
    Range,
    GatheringSpeed,
    GatheringEfficiency,
    CraftingSpeed,
}

impl StatKind {
    /// Its name among `words`, the name a game gives each stat.
    pub fn named(self, words: &StatMessages) -> &str {
        match self {
            StatKind::Health => words.health(),
            StatKind::MaxHealth => words.max_health(),
            StatKind::Mana => words.mana(),
            StatKind::MaxMana => words.max_mana(),
            StatKind::Damage => words.damage(),
            StatKind::AttackSpeed => words.attack_speed(),
            StatKind::AttackDelay => words.attack_delay(),
            StatKind::MovementSpeed => words.movement_speed(),
            StatKind::Range => words.range(),
            StatKind::GatheringSpeed => words.gathering_speed(),
            StatKind::GatheringEfficiency => words.gathering_efficiency(),
            StatKind::CraftingSpeed => words.crafting_speed(),
        }
    }

    pub const fn of(self, value: f32) -> Stat {
        Stat { kind: self, value }
    }

    pub const fn unit(self) -> Option<&'static str> {
        match self {
            StatKind::AttackDelay => Some("ms"),
            StatKind::MovementSpeed => Some("m/s"),
            StatKind::Range => Some("m"),
            StatKind::GatheringEfficiency | StatKind::CraftingSpeed => Some("%"),
            StatKind::Health
            | StatKind::MaxHealth
            | StatKind::Mana
            | StatKind::MaxMana
            | StatKind::Damage
            | StatKind::AttackSpeed
            | StatKind::GatheringSpeed => None,
        }
    }
}

messages! {
    /// What the game calls each stat.
    pub struct StatMessages {
        health,
        max_health,
        mana,
        max_mana,
        damage,
        attack_speed,
        attack_delay,
        movement_speed,
        range,
        gathering_speed,
        gathering_efficiency,
        crafting_speed,
        /// The player sheet's row for the player's name.
        name,
        /// The player sheet's row for the player's class.
        class,
        /// The player sheet's row for the player's level.
        level,
        /// A number `{value}` the player sheet shows for a stat or the level.
        figure(value: Number),
        /// A number `{value}` the player sheet shows for a stat measured in a unit, `{unit}`.
        figure_with_unit(unit: Text, value: Number),
    }
}

/// What an actor spends and regains, each held within a maximum of its own. Out of health an
/// actor falls; out of mana it only goes without.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Vital {
    #[default]
    Health,
    Mana,
}

impl Vital {
    pub const BOTH: [Vital; 2] = [Vital::Health, Vital::Mana];

    pub fn is_health(&self) -> bool {
        *self == Vital::Health
    }

    /// The stat that says how much of it an actor holds.
    pub const fn held(self) -> StatKind {
        match self {
            Vital::Health => StatKind::Health,
            Vital::Mana => StatKind::Mana,
        }
    }

    /// The stat that says how much of it an actor holds at most.
    pub const fn most(self) -> StatKind {
        match self {
            Vital::Health => StatKind::MaxHealth,
            Vital::Mana => StatKind::MaxMana,
        }
    }
}

#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Stat {
    #[serde(rename = "stat")]
    pub kind: StatKind,
    pub value: f32,
}

#[derive(Component, JsonSchema, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(transparent)]
#[schemars(transform = schema_with_units)]
pub struct Stats(pub BTreeMap<StatKind, f32>);

impl Stats {
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

/// A system's say in an actor's effective stats, on top of its own `Stats`, pushed onto the
/// modifiers gathered so far; registered with `source`.
pub type StatSource = fn(&World, Entity, &mut dyn FnMut(Stat));

#[derive(Resource, Default)]
pub struct Sources(Vec<StatSource>);

pub fn register(app: &mut App) {
    use bevy_replication::AppReplicationExt;
    app.replicate::<Stats>();
}

pub fn source(world: &mut World, source: StatSource) {
    world.resource_mut::<Sources>().0.push(source);
}

/// Hands `into` every modifier on the entity's stats, source by source.
pub fn modifiers(world: &World, entity: Entity, into: &mut dyn FnMut(Stat)) {
    for source in &world.resource::<Sources>().0 {
        source(world, entity, into);
    }
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
    let mut delta = 0.0;
    modifiers(world, entity, &mut |stat| {
        if stat.kind == kind {
            delta += stat.value;
        }
    });
    base(world, entity, kind) + delta
}

pub fn effective_all(world: &World, entity: Entity) -> Stats {
    let mut modifiers = Vec::new();
    self::modifiers(world, entity, &mut |stat| modifiers.push(stat));
    Stats(
        StatKind::VARIANTS
            .iter()
            .map(|&kind| {
                let delta: f32 = modifiers
                    .iter()
                    .filter(|stat| stat.kind == kind)
                    .map(|stat| stat.value)
                    .sum();
                (kind, base(world, entity, kind) + delta)
            })
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
            property.insert(SchemaMark::Unit.key().to_owned(), unit.into());
        }
    }
}

pub fn scripting(vocabulary: &mut Vocabulary) {
    query::scripting(vocabulary);
}
