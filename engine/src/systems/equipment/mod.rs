pub mod widget;

use std::collections::BTreeMap;

use bevy_app::App;
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{IntoStaticStr, VariantArray};

use crate::core::content::Content;
use crate::systems::drain_requests;
use crate::systems::item::{Inventory, ItemId, ItemKind, ItemStack};
use crate::systems::notification;
use crate::systems::player::{may_command, sender_player};
use crate::systems::script::condition;

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;
    app.replicate::<Equipment>()
        .add_client_message::<UnequipRequest>(Channel::Ordered);
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Equipment {
    pub slots: BTreeMap<EquipmentSlot, ItemId>,
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct UnequipRequest {
    pub slot: EquipmentSlot,
}

#[derive(
    JsonSchema,
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
#[serde(rename_all = "snake_case")]
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
}

pub fn equip_held(world: &mut World, player: Entity, item: ItemId) -> Result<(), String> {
    let slot = world
        .get::<Inventory>(player)
        .and_then(|inventory| inventory.slots.iter().position(|stack| stack.item == item))
        .ok_or_else(|| {
            format!(
                "{} is not in your bag",
                item.get(world.resource::<Content>()).name
            )
        })?;
    equip(world, player, slot)
}

pub fn equip(world: &mut World, player: Entity, inv_slot: usize) -> Result<(), String> {
    let content = world.resource::<Content>().clone();
    let item = world
        .get::<Inventory>(player)
        .and_then(|inventory| inventory.slots.get(inv_slot).map(|slot| slot.item))
        .ok_or_else(|| "Nothing there".to_owned())?;
    let ItemKind::Equipment {
        slot: into,
        requires,
        ..
    } = &item.get(&content).kind
    else {
        return Err(format!("{} can't be worn", item.get(&content).name));
    };
    if let Some(unmet) = condition::first_unmet(world, player, requires) {
        return Err(format!("{} needs {unmet}", item.get(&content).name));
    }
    let occupant = world
        .get::<Equipment>(player)
        .and_then(|equipment| equipment.slots.get(into).copied());
    let returned: Vec<ItemStack> = occupant
        .map(|occupant| ItemStack::new(occupant, 1))
        .into_iter()
        .collect();
    world
        .get_mut::<Inventory>(player)
        .ok_or_else(|| "Has no bag".to_owned())?
        .exchange(&content, &[ItemStack::new(item, 1)], &returned)
        .map_err(|refusal| refusal.describe(&content))?;
    if let Some(mut equipment) = world.get_mut::<Equipment>(player) {
        equipment.slots.insert(*into, item);
    }
    Ok(())
}

pub fn unequip_slot(world: &mut World, player: Entity, slot: EquipmentSlot) -> Result<(), String> {
    let content = world.resource::<Content>().clone();
    let item = world
        .get::<Equipment>(player)
        .and_then(|equipment| equipment.slots.get(&slot).copied())
        .ok_or_else(|| format!("Nothing worn as {}", slot.label()))?;
    let mut inventory = world
        .get_mut::<Inventory>(player)
        .ok_or_else(|| "Has no bag".to_owned())?;
    inventory
        .exchange(&content, &[], &[ItemStack::new(item, 1)])
        .map_err(|refusal| refusal.describe(&content))?;
    if let Some(mut equipment) = world.get_mut::<Equipment>(player) {
        equipment.slots.remove(&slot);
    }
    Ok(())
}

pub fn unequip(world: &mut World) {
    for request in drain_requests::<UnequipRequest>(world) {
        let Some(player) = sender_player(world, request.client_id) else {
            continue;
        };
        if !may_command(world, player) {
            continue;
        }
        if let Err(refusal) = unequip_slot(world, player, request.message.slot) {
            notification::error(world, player, refusal);
        }
    }
}
