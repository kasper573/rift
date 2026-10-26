pub mod widget;

use std::collections::BTreeMap;

use bevy_app::App;
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::Content;
use crate::systems::actor::stance::HandIndex;
use crate::systems::actor::wearing::{self, Wearing};
use crate::systems::item::{self, Inventory, ItemId, ItemKind, ItemStack, Wear};
use crate::systems::player::player_requests;
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

/// A place a player wears an item: one of its body's hands, in the skeleton's order, or its head,
/// which counts for stats and is never drawn.
#[derive(
    JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord,
)]
#[serde(rename_all = "snake_case")]
pub enum EquipmentSlot {
    Hand(HandIndex),
    Head,
}

pub fn wears(world: &World, actor: Entity, item: ItemId) -> bool {
    worn(world, actor).any(|worn| worn == item)
}

/// Every item worn, each once however many hands it takes.
pub fn worn(world: &World, actor: Entity) -> impl Iterator<Item = ItemId> + '_ {
    let mut seen: Vec<ItemId> = Vec::new();
    world
        .get::<Equipment>(actor)
        .into_iter()
        .flat_map(|equipment| equipment.slots.values().copied())
        .filter(move |item| {
            let first = !seen.contains(item);
            seen.push(*item);
            first
        })
}

pub fn in_slot(world: &World, actor: Entity, slot: EquipmentSlot) -> Option<ItemId> {
    world
        .get::<Equipment>(actor)
        .and_then(|equipment| equipment.slots.get(&slot).copied())
}

/// The hand slots a player's body gives it: one per hand of its skeleton, and one still on a body
/// with none, where a held item is worn and never shown.
pub fn hand_slots(world: &World, player: Entity) -> usize {
    wearing::hands_of(world, player).len().max(1)
}

pub fn equip_held(world: &mut World, player: Entity, item: ItemId) -> Result<(), String> {
    let slot = item::held_slot(world, player, item)?;
    equip(world, player, slot)
}

/// Wears the item in the bag's slot `inv_slot` where it goes: a head item on the head, a held item
/// where its stance's hand says, in the same exchange returning to the bag what it displaces.
pub fn equip(world: &mut World, player: Entity, inv_slot: usize) -> Result<(), String> {
    let content = world.resource::<Content>().clone();
    let item = world
        .get::<Inventory>(player)
        .and_then(|inventory| inventory.slots.get(inv_slot).map(|slot| slot.item))
        .ok_or_else(|| "Nothing there".to_owned())?;
    let ItemKind::Equipment {
        slot: wear,
        requires,
        ..
    } = &item.get(&content).kind
    else {
        return Err(format!("{} can't be worn", item.get(&content).name));
    };
    if let Some(unmet) = condition::first_unmet(world, player, requires) {
        return Err(format!("{} needs {unmet}", item.get(&content).name));
    }
    let worn = world.get::<Equipment>(player).cloned().unwrap_or_default();
    let (slots, returned): (BTreeMap<EquipmentSlot, ItemId>, Vec<ItemId>) = match wear {
        Wear::Head => {
            let mut slots = worn.slots.clone();
            let returned = slots.insert(EquipmentSlot::Head, item);
            (slots, returned.into_iter().collect())
        }
        Wear::Hand { .. } => {
            let held = hands(&worn);
            let placement = wearing::place(&content, hand_slots(world, player), &held, item);
            let mut slots: BTreeMap<EquipmentSlot, ItemId> = worn
                .slots
                .iter()
                .filter(|(slot, _)| !matches!(slot, EquipmentSlot::Hand(_)))
                .map(|(&slot, &item)| (slot, item))
                .collect();
            slots.extend(
                placement
                    .hands
                    .into_iter()
                    .map(|(hand, item)| (EquipmentSlot::Hand(hand), item)),
            );
            (slots, placement.returned)
        }
    };
    let returned: Vec<ItemStack> = returned
        .into_iter()
        .map(|item| ItemStack::new(item, 1))
        .collect();
    world
        .get_mut::<Inventory>(player)
        .ok_or_else(|| "Has no bag".to_owned())?
        .exchange(&content, &[ItemStack::new(item, 1)], &returned)
        .map_err(|refusal| refusal.describe(&content))?;
    world.entity_mut(player).insert(Equipment { slots });
    Ok(())
}

/// Where items put on one after another are worn, each as equipping puts it: what it displaced from
/// the hands is not put back on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PutOn {
    pub slots: BTreeMap<EquipmentSlot, ItemId>,
    /// What a later item displaced, with the item that displaced it.
    pub displaced: Vec<(ItemId, ItemId)>,
}

pub fn put_on(content: &Content, hand_slots: usize, items: &[ItemId]) -> PutOn {
    let mut put = PutOn::default();
    for &item in items {
        match item.get(content).wear() {
            Some(Wear::Head) => {
                if let Some(there) = put.slots.insert(EquipmentSlot::Head, item) {
                    put.displaced.push((there, item));
                }
            }
            Some(Wear::Hand { .. }) => {
                let held = hands(&Equipment {
                    slots: put.slots.clone(),
                });
                let placement = wearing::place(content, hand_slots, &held, item);
                put.slots
                    .retain(|slot, _| !matches!(slot, EquipmentSlot::Hand(_)));
                put.slots.extend(
                    placement
                        .hands
                        .into_iter()
                        .map(|(hand, item)| (EquipmentSlot::Hand(hand), item)),
                );
                put.displaced
                    .extend(placement.returned.into_iter().map(|there| (there, item)));
            }
            None => {}
        }
    }
    put
}

/// What a list of items put on in order must be: equipment, every one still worn once the last is
/// on.
pub fn check_put_on(
    content: &Content,
    hand_slots: usize,
    items: &[ItemId],
    pointer: &str,
    check: &mut crate::core::content::RowCheck,
) {
    for (index, &item) in items.iter().enumerate() {
        if item.get(content).wear().is_none() {
            check.error(
                format!("{pointer}/{index}"),
                "not wearable",
                item.path(content),
                "an equipment item",
            );
        }
    }
    for (there, by) in put_on(content, hand_slots, items).displaced {
        check.error(
            pointer,
            "displaced",
            format!(
                "{} takes the place of {}",
                by.path(content),
                there.path(content)
            ),
            format!("items worn together, in at most {hand_slots} hands and a head"),
        );
    }
}

/// Takes off what is worn in `slot`, from every hand it takes, back into the bag.
pub fn unequip_slot(world: &mut World, player: Entity, slot: EquipmentSlot) -> Result<(), String> {
    let content = world.resource::<Content>().clone();
    let item = in_slot(world, player, slot).ok_or_else(|| "Nothing worn there".to_owned())?;
    let mut inventory = world
        .get_mut::<Inventory>(player)
        .ok_or_else(|| "Has no bag".to_owned())?;
    inventory
        .exchange(&content, &[], &[ItemStack::new(item, 1)])
        .map_err(|refusal| refusal.describe(&content))?;
    if let Some(equipment) = world.get::<Equipment>(player) {
        let mut slots = equipment.slots.clone();
        slots.retain(|worn, there| {
            *worn != slot && !(matches!(worn, EquipmentSlot::Hand(_)) && *there == item)
        });
        world.entity_mut(player).insert(Equipment { slots });
    }
    Ok(())
}

/// Takes `item` off, from wherever it is worn.
pub fn unequip_item(world: &mut World, player: Entity, item: ItemId) -> Result<(), String> {
    let slot = world
        .get::<Equipment>(player)
        .and_then(|equipment| {
            equipment
                .slots
                .iter()
                .find(|(_, worn)| **worn == item)
                .map(|(slot, _)| *slot)
        })
        .ok_or_else(|| format!("{} is not worn", item.get(world.resource::<Content>()).name))?;
    unequip_slot(world, player, slot)
}

/// Keeps what a player's body shows equal to its equipped hands, from the moment it is spawned
/// equipped to every change of what it wears, each of which puts its equipment anew.
pub fn reflect(
    insert: On<Insert<Equipment>>,
    players: Query<(&Equipment, Option<&Wearing>)>,
    mut commands: Commands,
) {
    let Ok((equipment, wearing)) = players.get(insert.entity) else {
        return;
    };
    let shown = Wearing {
        hands: hands(equipment),
    };
    if wearing != Some(&shown) {
        commands.entity(insert.entity).insert(shown);
    }
}

/// The item in each equipped hand.
pub fn hands(equipment: &Equipment) -> BTreeMap<HandIndex, ItemId> {
    equipment
        .slots
        .iter()
        .filter_map(|(slot, &item)| match slot {
            EquipmentSlot::Hand(hand) => Some((*hand, item)),
            EquipmentSlot::Head => None,
        })
        .collect()
}

pub fn unequip(world: &mut World) {
    player_requests::<UnequipRequest>(world, true, |world, player, request| {
        unequip_slot(world, player, request.slot)
    });
}
