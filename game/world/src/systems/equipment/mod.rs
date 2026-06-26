//! Equipment: the gear a player wears per [`EquipSlot`], its effects active only while worn. Items
//! declare a slot and optional [`RequirementKind`]s (job, level, or a stat floor) checked on equip.

mod job;
mod level;
mod stat;

pub use job::JobRequirement;
pub use level::LevelRequirement;
pub use stat::StatRequirement;

use std::collections::BTreeMap;

use bevy_app::App;
use bevy_ecs::prelude::*;
use enum_dispatch::enum_dispatch;
use serde::{Deserialize, Serialize};

use crate::core::table::Id;
use crate::systems::effect::{self, EffectCommand};
use crate::systems::item::{Inventory, ItemDef};
use crate::systems::player::sender_player;

/// A gate on equipping an item, one file per kind implementing [`Requirement`], dispatched by
/// [`RequirementKind`]. Adding a gate kind is a new file plus a variant — [`met`] matches none.
#[enum_dispatch]
pub trait Requirement {
    /// Whether `player` satisfies this gate.
    fn met(&self, world: &World, player: Entity) -> bool;
}

/// Stored in item defs (json only, never replicated), so the tagged representation is fine.
#[enum_dispatch(Requirement)]
#[derive(Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RequirementKind {
    Job(JobRequirement),
    Level(LevelRequirement),
    Stat(StatRequirement),
}

/// Whether `player` satisfies every gate.
pub fn met(world: &World, player: Entity, requirements: &[RequirementKind]) -> bool {
    requirements
        .iter()
        .all(|requirement| requirement.met(world, player))
}

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.replicate::<Equipment>()
        .add_client_message::<UnequipRequest>(Channel::Ordered);
    effect::source(app, equipped);
}

/// Effect source: every equipped item's effects, active while worn.
fn equipped(world: &World, entity: Entity) -> Vec<EffectCommand> {
    world
        .get::<Equipment>(entity)
        .map(|equipment| {
            equipment
                .slots
                .values()
                .flat_map(|item| item.get().effects.iter().cloned())
                .collect()
        })
        .unwrap_or_default()
}

#[derive(
    Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum EquipSlot {
    #[default]
    Weapon,
    Offhand,
    Head,
}

impl EquipSlot {
    pub const ALL: [EquipSlot; 3] = [EquipSlot::Weapon, EquipSlot::Offhand, EquipSlot::Head];

    pub fn label(self) -> &'static str {
        match self {
            EquipSlot::Weapon => "Weapon",
            EquipSlot::Offhand => "Offhand",
            EquipSlot::Head => "Head",
        }
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Equipment {
    pub slots: BTreeMap<EquipSlot, Id<ItemDef>>,
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct UnequipRequest {
    pub slot: EquipSlot,
}

/// Moves the inventory item at `inv_slot` into equipment slot `into` if `requirements` pass, sending
/// any occupant back to the inventory. Called by `items::use_item` when an equipment item is used.
pub fn equip(
    world: &mut World,
    player: Entity,
    inv_slot: usize,
    into: EquipSlot,
    requirements: &[RequirementKind],
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
