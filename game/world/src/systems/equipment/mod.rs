pub mod req;
pub mod slot;

pub use req::{JobRequirement, LevelRequirement, Requirement, StatRequirement, met};
pub use slot::{HeadSlot, OffhandSlot, Slot, SlotId, WeaponSlot};

use std::collections::BTreeMap;

use bevy_app::App;
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::table::Id;
use crate::systems::effect::{self, EffectCommand};
use crate::systems::item::{Inventory, ItemDef};
use crate::systems::player::sender_player;

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.replicate::<Equipment>()
        .add_client_message::<UnequipRequest>(Channel::Ordered);
    effect::source(app, equipped);
}

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

#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Equipment {
    pub slots: BTreeMap<SlotId, Id<ItemDef>>,
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct UnequipRequest {
    pub slot: SlotId,
}

pub fn equip(
    world: &mut World,
    player: Entity,
    inv_slot: usize,
    into: SlotId,
    requirements: &[Box<dyn Requirement>],
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
