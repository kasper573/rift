pub mod widget;

use std::collections::BTreeMap;

use bevy_app::App;
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::content::Content;
use crate::systems::actor::held::{self, Held};
use crate::systems::actor::skeleton::EquipmentSlot;
use crate::systems::actor::stance::HandIndex;
use crate::systems::actor::{ActorModel, appearance};
use crate::systems::item::{self, Inventory, ItemId, ItemKind, ItemStack, Wear};
use crate::systems::player::player_requests;
use crate::systems::script::condition;

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::{Channel, ClientMessageAppExt};

    use crate::systems::mirror::AppMirrorExt;
    app.replicate::<Equipment>()
        .add_client_message::<UnequipRequest>(Channel::Ordered)
        .add_client_message::<HoldRequest>(Channel::Ordered);
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Equipment {
    pub slots: BTreeMap<EquipmentPlace, ItemId>,
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct UnequipRequest {
    pub slot: EquipmentPlace,
}

/// Holds the item in the bag's slot `slot` in `hand`, whichever hand its stance's clips were made
/// for.
#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct HoldRequest {
    pub slot: u32,
    pub hand: HandIndex,
}

/// A place a player wears an item: one of its body's hands, in the skeleton's order, or one of the
/// slots its skeleton lists.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum EquipmentPlace {
    Hand(HandIndex),
    Slot(EquipmentSlot),
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

pub fn in_slot(world: &World, actor: Entity, slot: &EquipmentPlace) -> Option<ItemId> {
    world
        .get::<Equipment>(actor)
        .and_then(|equipment| equipment.slots.get(slot).copied())
}

/// The hand slots a player's body gives it: one per hand of its skeleton, and one still on a body
/// with none, where a held item is worn and never shown.
pub fn hand_slots(world: &World, player: Entity) -> usize {
    held::hands_of(world, player).len().max(1)
}

pub fn equip_from_bag(world: &mut World, player: Entity, item: ItemId) -> Result<(), String> {
    let slot = item::bag_slot(world, player, item)?;
    equip(world, player, slot, None)
}

/// Wears the item in the bag's slot `inv_slot` where it goes: a worn item in its slot, when the
/// body has it, a held item in `hand` or else where its stance's hand says, in the same exchange
/// returning to the bag what it displaces.
pub fn equip(
    world: &mut World,
    player: Entity,
    inv_slot: usize,
    hand: Option<HandIndex>,
) -> Result<(), String> {
    let content = world.resource::<Content>().clone();
    let item = world
        .get::<Inventory>(player)
        .and_then(|inventory| inventory.slots.get(inv_slot).map(|slot| slot.item))
        .ok_or_else(|| "Nothing there".to_owned())?;
    let ItemKind::Equipment { wear, requires, .. } = &item.get(&content).kind else {
        return Err(format!("{} can't be worn", item.get(&content).name));
    };
    if let Some(unmet) = condition::first_unmet(world, player, requires) {
        return Err(format!("{} needs {unmet}", item.get(&content).name));
    }
    let worn = world.get::<Equipment>(player).cloned().unwrap_or_default();
    let (slots, returned): (BTreeMap<EquipmentPlace, ItemId>, Vec<ItemId>) = match wear {
        Wear::Worn { slot, .. } => {
            if !body_slots(world, player).contains(slot) {
                return Err(format!(
                    "{} can't be worn by this body",
                    item.get(&content).name
                ));
            }
            let mut slots = worn.slots.clone();
            let returned = slots.insert(EquipmentPlace::Slot(slot.clone()), item);
            (slots, returned.into_iter().collect())
        }
        Wear::Held { .. } => {
            let held = hands(&worn);
            let placement = held::place(&content, hand_slots(world, player), &held, item, hand);
            let mut slots: BTreeMap<EquipmentPlace, ItemId> = worn
                .slots
                .iter()
                .filter(|(slot, _)| !matches!(slot, EquipmentPlace::Hand(_)))
                .map(|(slot, &item)| (slot.clone(), item))
                .collect();
            slots.extend(
                placement
                    .hands
                    .into_iter()
                    .map(|(hand, item)| (EquipmentPlace::Hand(hand), item)),
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

/// The slots items are worn in on the actor's body: its skeleton's.
pub fn body_slots(world: &World, actor: Entity) -> Vec<EquipmentSlot> {
    appearance::model_of(world, actor)
        .map_or_else(Vec::new, |model| model.skeleton().def.slots.clone())
}

/// Where items put on one after another are worn on `body`, each as equipping puts it: what it
/// displaced is not put back on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PutOn {
    pub slots: BTreeMap<EquipmentPlace, ItemId>,
    /// What a later item displaced, with the item that displaced it.
    pub displaced: Vec<(ItemId, ItemId)>,
    /// What is worn in a slot the body does not have, and so is not put on.
    pub unfit: Vec<ItemId>,
}

pub fn put_on(content: &Content, body: &ActorModel, items: &[ItemId]) -> PutOn {
    let hand_slots = body.hands().len().max(1);
    let body_slots = &body.skeleton().def.slots;
    let mut put = PutOn::default();
    for &item in items {
        match item.get(content).wear() {
            Some(Wear::Worn { slot, .. }) if !body_slots.contains(slot) => put.unfit.push(item),
            Some(Wear::Worn { slot, .. }) => {
                if let Some(there) = put.slots.insert(EquipmentPlace::Slot(slot.clone()), item) {
                    put.displaced.push((there, item));
                }
            }
            Some(Wear::Held { .. }) => {
                let held = hands(&Equipment {
                    slots: put.slots.clone(),
                });
                let placement = held::place(content, hand_slots, &held, item, None);
                put.slots
                    .retain(|slot, _| !matches!(slot, EquipmentPlace::Hand(_)));
                put.slots.extend(
                    placement
                        .hands
                        .into_iter()
                        .map(|(hand, item)| (EquipmentPlace::Hand(hand), item)),
                );
                put.displaced
                    .extend(placement.returned.into_iter().map(|there| (there, item)));
            }
            None => {}
        }
    }
    put
}

/// What a list of items put on in order must be: equipment, each worn in a slot the body has, every
/// one still worn once the last is on.
pub fn check_put_on(
    content: &Content,
    body: &ActorModel,
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
    let put = put_on(content, body, items);
    for item in put.unfit {
        check.error(
            pointer,
            "no such slot",
            format!("{} is worn in a slot the body lacks", item.path(content)),
            format!(
                "items worn in the slots of the body's skeleton: {}",
                body.skeleton()
                    .def
                    .slots
                    .iter()
                    .map(EquipmentSlot::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        );
    }
    for (there, by) in put.displaced {
        check.error(
            pointer,
            "displaced",
            format!(
                "{} takes the place of {}",
                by.path(content),
                there.path(content)
            ),
            format!(
                "items worn together, in at most {} hands and one a slot",
                body.hands().len().max(1)
            ),
        );
    }
}

/// Takes off what is worn in `slot`, from every hand it takes, back into the bag.
pub fn unequip_slot(world: &mut World, player: Entity, slot: EquipmentPlace) -> Result<(), String> {
    let content = world.resource::<Content>().clone();
    let item = in_slot(world, player, &slot).ok_or_else(|| "Nothing worn there".to_owned())?;
    let mut inventory = world
        .get_mut::<Inventory>(player)
        .ok_or_else(|| "Has no bag".to_owned())?;
    inventory
        .exchange(&content, &[], &[ItemStack::new(item, 1)])
        .map_err(|refusal| refusal.describe(&content))?;
    if let Some(equipment) = world.get::<Equipment>(player) {
        let mut slots = equipment.slots.clone();
        slots.retain(|worn, there| {
            *worn != slot && !(matches!(worn, EquipmentPlace::Hand(_)) && *there == item)
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
                .map(|(slot, _)| slot.clone())
        })
        .ok_or_else(|| format!("{} is not worn", item.get(world.resource::<Content>()).name))?;
    unequip_slot(world, player, slot)
}

/// Keeps what a player's body shows equal to its equipped hands, from the moment it is spawned
/// equipped to every change of what it wears, each of which puts its equipment anew.
pub fn reflect(
    insert: On<Insert<Equipment>>,
    players: Query<(&Equipment, Option<&Held>)>,
    mut commands: Commands,
) {
    let Ok((equipment, wearing)) = players.get(insert.entity) else {
        return;
    };
    let shown = Held {
        hands: hands(equipment),
    };
    if wearing != Some(&shown) {
        commands.entity(insert.entity).insert(shown);
    }
}

/// The item worn in each slot of the body.
pub fn worn_slots(equipment: &Equipment) -> impl Iterator<Item = (&EquipmentSlot, &ItemId)> {
    equipment
        .slots
        .iter()
        .filter_map(|(place, item)| match place {
            EquipmentPlace::Slot(slot) => Some((slot, item)),
            EquipmentPlace::Hand(_) => None,
        })
}

/// The item in each equipped hand.
pub fn hands(equipment: &Equipment) -> BTreeMap<HandIndex, ItemId> {
    equipment
        .slots
        .iter()
        .filter_map(|(slot, &item)| match slot {
            EquipmentPlace::Hand(hand) => Some((*hand, item)),
            EquipmentPlace::Slot(_) => None,
        })
        .collect()
}

pub fn hold(world: &mut World) {
    player_requests::<HoldRequest>(world, true, |world, player, request| {
        equip(world, player, request.slot as usize, Some(request.hand))
    });
}

pub fn unequip(world: &mut World) {
    player_requests::<UnequipRequest>(world, true, |world, player, request| {
        unequip_slot(world, player, request.slot)
    });
}
