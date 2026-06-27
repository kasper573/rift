use std::collections::BTreeMap;

use bevy_app::App;
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};
use strum::{IntoStaticStr, VariantArray};

use crate::data;
use crate::systems::effect::{self, Effect};
use crate::systems::item::Inventory;
use crate::systems::job;
use crate::systems::player::sender_player;
use crate::systems::stat::{self, StatKind};

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.replicate::<Equipment>()
        .add_client_message::<UnequipRequest>(Channel::Ordered);
    effect::source(app, equipped);
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Equipment {
    pub slots: BTreeMap<EquipmentSlot, data::item::Id>,
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct UnequipRequest {
    pub slot: EquipmentSlot,
}

#[derive(
    Serialize,
    Deserialize,
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Default,
    VariantArray,
    IntoStaticStr,
)]
pub enum EquipmentSlot {
    #[default]
    Weapon,
    Offhand,
    Head,
}

impl EquipmentSlot {
    pub fn label(self) -> &'static str {
        self.into()
    }

    pub fn all() -> &'static [EquipmentSlot] {
        EquipmentSlot::VARIANTS
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Requirement {
    Level(u32),
    Job(data::job::Id),
    Stat { stat: StatKind, min: f32 },
}

impl Requirement {
    pub fn met(self, world: &World, player: Entity) -> bool {
        match self {
            Requirement::Level(level) => job::level(world, player) >= level,
            Requirement::Job(job) => world
                .get::<job::Job>(player)
                .is_some_and(|held| held.def == job),
            Requirement::Stat { stat, min } => stat::effective(world, player, stat) >= min,
        }
    }
}

pub fn met(world: &World, player: Entity, requirements: &[Requirement]) -> bool {
    requirements
        .iter()
        .all(|requirement| requirement.met(world, player))
}

pub fn equip(
    world: &mut World,
    player: Entity,
    inv_slot: usize,
    into: EquipmentSlot,
    requirements: &[Requirement],
) {
    let Some(item) = world
        .get::<Inventory>(player)
        .and_then(|inventory| inventory.slots.get(inv_slot).map(|slot| slot.item))
    else {
        return;
    };
    if !met(world, player, requirements) {
        return;
    }
    let occupant = world
        .get::<Equipment>(player)
        .and_then(|equipment| equipment.slots.get(&into).copied());
    if let Some(mut inventory) = world.get_mut::<Inventory>(player) {
        inventory.slots.remove(inv_slot);
        if let Some(occupant) = occupant {
            inventory.add(occupant, 1);
        }
    }
    if let Some(mut equipment) = world.get_mut::<Equipment>(player) {
        equipment.slots.insert(into, item);
    }
}

pub fn unequip(world: &mut World) {
    for request in crate::systems::requests::<UnequipRequest>(world) {
        let Some(player) = sender_player(world, request.client_id) else {
            continue;
        };
        let slot = request.message.slot;
        let Some(item) = world
            .get::<Equipment>(player)
            .and_then(|equipment| equipment.slots.get(&slot).copied())
        else {
            continue;
        };
        if world
            .get::<Inventory>(player)
            .is_none_or(|inventory| inventory.capacity_for(item) < 1)
        {
            continue;
        }
        if let Some(mut equipment) = world.get_mut::<Equipment>(player) {
            equipment.slots.remove(&slot);
        }
        if let Some(mut inventory) = world.get_mut::<Inventory>(player) {
            inventory.add(item, 1);
        }
    }
}

fn equipped(world: &World, entity: Entity) -> Vec<Effect> {
    world
        .get::<Equipment>(entity)
        .map(|equipment| {
            equipment
                .slots
                .values()
                .flat_map(|item| item.get().effects.iter().copied())
                .collect()
        })
        .unwrap_or_default()
}
