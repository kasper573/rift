pub mod category;
pub mod query;
pub mod widget;

use crate::core::script::Vocabulary;
use std::collections::BTreeMap;

use bevy_app::App;
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::content::Content;
use crate::core::text::LineText;
use crate::systems::actor::held::{self, Held};
use crate::systems::actor::skeleton::EquipmentSlot;
use crate::systems::actor::stance::HandIndex;
use crate::systems::actor::{ActorModel, appearance};
use crate::systems::item::{self, Inventory, ItemId, ItemKind, ItemStack, Wear};
use crate::systems::mention::Mention;
use crate::systems::messages::{self, messages};
use crate::systems::player::RequestNeeds;
use crate::systems::player::player_requests;
use crate::systems::requirement;
use crate::systems::resource::Working;
use schemars::JsonSchema;

pub fn register(app: &mut App) {
    use bevy_replication::AppMessageExt;

    use bevy_replication::AppReplicationExt;
    app.replicate::<Equipment>()
        .add_client_message::<UnequipRequest>()
        .add_client_message::<EquipRequest>();
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Equipment {
    pub slots: BTreeMap<EquipmentPlace, ItemId>,
}

/// Takes off what is worn in `slot`, into the bag's slot `into` when one is named, as
/// `unequip_slot` puts it there.
#[derive(JsonSchema, Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct UnequipRequest {
    pub slot: EquipmentPlace,
    pub into: Option<u32>,
}

/// Puts on the item in the bag's slot `slot`: a held one in `hand` when one is named, whichever
/// hand its stance's clips were made for. Never uses the item: one that isn't worn is refused.
#[derive(JsonSchema, Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct EquipRequest {
    pub slot: u32,
    pub hand: Option<HandIndex>,
}

/// A place a character wears an item: one of its body's hands, in the skeleton's order, one of the
/// slots its skeleton lists, or the one place every body keeps a tool.
#[derive(
    JsonSchema, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord,
)]
#[serde(rename_all = "snake_case")]
pub enum EquipmentPlace {
    Hand(HandIndex),
    Slot(EquipmentSlot),
    Tool,
}

/// What a character has on them: their bag, and what they wear.
#[derive(Clone, Copy, Default)]
pub struct Belongings<'a> {
    bag: Option<&'a Inventory>,
    worn: Option<&'a Equipment>,
}

impl<'a> Belongings<'a> {
    pub fn by(world: &'a World, actor: impl Into<Option<Entity>>) -> Belongings<'a> {
        let actor = actor.into();
        Belongings {
            bag: actor.and_then(|actor| world.get(actor)),
            worn: actor.and_then(|actor| world.get(actor)),
        }
    }

    pub fn of(bag: Option<&'a Inventory>, worn: Option<&'a Equipment>) -> Belongings<'a> {
        Belongings { bag, worn }
    }

    /// How many of `item` they have. One worn counts once, however many hands it takes.
    pub fn count(&self, item: ItemId) -> u32 {
        let worn = self
            .worn
            .is_some_and(|worn| worn.slots.values().any(|worn| *worn == item));
        self.bag
            .map_or(0, |bag| bag.count(item))
            .saturating_add(u32::from(worn))
    }
}

/// Takes `stacks` from what `actor` has on them: from their bag, and from what they wear where the
/// bag falls short. Refuses, taking nothing, when they have too few.
pub fn take_belongings(
    world: &mut World,
    actor: Entity,
    stacks: &[ItemStack],
) -> Result<(), LineText> {
    let (worn, bagged) = worn_share(world, actor, stacks);
    item::effects::exchange(world, actor, &bagged, &[])?;
    discard_worn(world, actor, &worn);
    Ok(())
}

/// Takes as much of `stacks` as `actor` has, from their bag and from what they wear where
/// the bag falls short.
pub fn take_belongings_up_to(world: &mut World, actor: Entity, stacks: &[ItemStack]) {
    let (worn, bagged) = worn_share(world, actor, stacks);
    if let Some(mut bag) = world.get_mut::<Inventory>(actor) {
        bag.take_up_to(&bagged);
    }
    discard_worn(world, actor, &worn);
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

/// Where the actor wears the item, the first of its places when it takes several.
pub fn place_of(world: &World, actor: Entity, item: ItemId) -> Option<EquipmentPlace> {
    world.get::<Equipment>(actor).and_then(|equipment| {
        equipment
            .slots
            .iter()
            .find(|(_, worn)| **worn == item)
            .map(|(place, _)| place.clone())
    })
}

pub fn in_slot(world: &World, actor: Entity, slot: &EquipmentPlace) -> Option<ItemId> {
    world
        .get::<Equipment>(actor)
        .and_then(|equipment| equipment.slots.get(slot).copied())
}

/// The hand slots a character's body gives it: one per hand of its skeleton, and one still on a body
/// with none, where a held item is worn and never shown.
pub fn hand_slots(world: &World, player: Entity) -> usize {
    held::hands_of(world, player).len().max(1)
}

pub fn equip_from_bag(world: &mut World, player: Entity, item: ItemId) -> Result<(), LineText> {
    let slot = item::bag_slot(world, player, item)?;
    equip(world, player, slot, None)
}

/// Wears the item in the bag's slot `inv_slot` where it goes: a worn item in its slot, when the
/// body has it, a tool in the tool slot, a held item in `hand` or else where its stance's hand
/// says, in the same exchange returning to the bag what it displaces.
pub fn equip(
    world: &mut World,
    player: Entity,
    inv_slot: usize,
    hand: Option<HandIndex>,
) -> Result<(), LineText> {
    let content = world.resource::<Content>().clone();
    let messages = messages::of(&content);
    let item = world
        .get::<Inventory>(player)
        .and_then(|inventory| inventory.at(inv_slot).map(|stack| stack.item))
        .ok_or_else(|| LineText::from(messages.item.nothing_there()))?;
    let ItemKind::Equipment { wear, requires, .. } = &item.get(&content).kind else {
        return Err(messages
            .equipment
            .cannot_wear(Mention::Item(item).line(&content)));
    };
    if let Some(unmet) = requirement::first_unmet(world, player, requires) {
        return Err(messages
            .equipment
            .needs(Mention::Item(item).line(&content), unmet));
    }
    let worn = world.get::<Equipment>(player).cloned().unwrap_or_default();
    let (slots, returned): (BTreeMap<EquipmentPlace, ItemId>, Vec<ItemId>) = match wear {
        Wear::Worn { slot, .. } => {
            if !body_slots(world, player).contains(slot) {
                return Err(messages
                    .equipment
                    .body_cannot_wear(Mention::Item(item).line(&content)));
            }
            let mut slots = worn.slots.clone();
            let returned = slots.insert(EquipmentPlace::Slot(slot.clone()), item);
            (slots, returned.into_iter().collect())
        }
        Wear::Tool { .. } => {
            let mut slots = worn.slots.clone();
            let returned = slots.insert(EquipmentPlace::Tool, item);
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
    item::effects::exchange(world, player, &[ItemStack::new(item, 1)], &returned)?;
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
            Some(Wear::Tool { .. }) => {
                if let Some(there) = put.slots.insert(EquipmentPlace::Tool, item) {
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
                "items worn together, in at most {} hands, one a slot and one tool",
                body.hands().len().max(1)
            ),
        );
    }
}

/// Takes off what is worn in `slot`, from every hand it takes, back into the bag. Named a slot
/// of the bag `into`, it lands there when that slot is empty, and swaps with what the slot
/// holds when that is worn in `slot`; otherwise it goes where new items go.
pub fn unequip_slot(
    world: &mut World,
    player: Entity,
    slot: EquipmentPlace,
    into: Option<usize>,
) -> Result<(), LineText> {
    let content = world.resource::<Content>().clone();
    let messages = messages::of(&content);
    let item = in_slot(world, player, &slot)
        .ok_or_else(|| LineText::from(messages.equipment.nothing_worn()))?;
    let bag = Inventory::of(world, player);
    let there = into.and_then(|into| bag.slots.get(into).map(|held| (into, *held)));
    match there {
        Some((into, Some(held))) if worn_at(&content, held.item, &slot) => {
            let hand = match slot {
                EquipmentPlace::Hand(hand) => Some(hand),
                EquipmentPlace::Slot(_) | EquipmentPlace::Tool => None,
            };
            equip(world, player, into, hand)?;
            return land_in(world, player, &content, &bag, item, into);
        }
        Some((into, None)) => {
            let mut bag = world
                .get_mut::<Inventory>(player)
                .expect("the bag read a moment ago is still there");
            bag.slots[into] = Some(ItemStack::new(item, 1));
        }
        _ => item::effects::exchange(world, player, &[], &[ItemStack::new(item, 1)])?,
    }
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
pub fn unequip_item(world: &mut World, player: Entity, item: ItemId) -> Result<(), LineText> {
    let slot = world
        .get::<Equipment>(player)
        .and_then(|equipment| {
            equipment
                .slots
                .iter()
                .find(|(_, worn)| **worn == item)
                .map(|(slot, _)| slot.clone())
        })
        .ok_or_else(|| {
            let content = world.resource::<Content>();
            messages::of(content)
                .equipment
                .not_worn(Mention::Item(item).line(content))
        })?;
    unequip_slot(world, player, slot, None)
}

/// The items of `stacks` that `actor` wears and whose bag holds too few of, and what is left of
/// `stacks` for the bag to give once those are taken off them.
fn worn_share(world: &World, actor: Entity, stacks: &[ItemStack]) -> (Vec<ItemId>, Vec<ItemStack>) {
    let bag = Inventory::of(world, actor);
    let mut worn = Vec::new();
    let mut bagged = Vec::new();
    for &stack in stacks {
        let short = bag.count(stack.item) < stack.count;
        let left = if short && wears(world, actor, stack.item) {
            worn.push(stack.item);
            stack.count - 1
        } else {
            stack.count
        };
        if left > 0 {
            bagged.push(ItemStack::new(stack.item, left));
        }
    }
    (worn, bagged)
}

/// Removes each of `items` from wherever `actor` wears it, without a place to land.
fn discard_worn(world: &mut World, actor: Entity, items: &[ItemId]) {
    if items.is_empty() {
        return;
    }
    if let Some(equipment) = world.get::<Equipment>(actor) {
        let mut slots = equipment.slots.clone();
        slots.retain(|_, worn| !items.contains(worn));
        world.entity_mut(actor).insert(Equipment { slots });
    }
}

/// Whether `item` is put on in `place` when it is equipped.
fn worn_at(content: &Content, item: ItemId, place: &EquipmentPlace) -> bool {
    match (item.get(content).wear(), place) {
        (Some(Wear::Worn { slot, .. }), EquipmentPlace::Slot(at)) => slot == at,
        (Some(Wear::Tool { .. }), EquipmentPlace::Tool) => true,
        (Some(Wear::Held { .. }), EquipmentPlace::Hand(_)) => true,
        _ => false,
    }
}

/// Moves `item`, which has just come back into the bag that was `before`, to its slot `into`
/// when that slot is empty now.
fn land_in(
    world: &mut World,
    player: Entity,
    content: &Content,
    before: &Inventory,
    item: ItemId,
    into: usize,
) -> Result<(), LineText> {
    let Some(mut bag) = world.get_mut::<Inventory>(player) else {
        return Ok(());
    };
    let landed = bag
        .slots
        .iter()
        .zip(&before.slots)
        .position(|(now, was)| now.is_some_and(|now| now.item == item) && now != was);
    match landed {
        Some(from) if bag.at(into).is_none() => bag
            .move_stack(content, from, into, item)
            .map_err(|changed| changed.line(content)),
        _ => Ok(()),
    }
}

/// Keeps what a character's body shows equal to its equipped hands, from the moment it is spawned
/// equipped to every change of what it wears, each of which puts its equipment anew.
pub fn reflect(insert: On<Insert<Equipment>>, world: &World, mut commands: Commands) {
    let working = world.get::<Working>(insert.entity).is_some();
    show(world, insert.entity, working, &mut commands);
}

/// Puts in the hands of `player` what they show: what they hold, or while it is `working` its tool
/// alone, in the hand the tool's stance names, everything held put away for as long.
pub fn show(world: &World, player: Entity, working: bool, commands: &mut Commands) {
    let Some(equipment) = world.get::<Equipment>(player) else {
        return;
    };
    let tool = equipment
        .slots
        .get(&EquipmentPlace::Tool)
        .filter(|_| working);
    let hands = match tool {
        Some(&tool) => {
            let content = world.resource::<Content>();
            let empty = BTreeMap::new();
            held::place(content, hand_slots(world, player), &empty, tool, None).hands
        }
        None => hands(equipment),
    };
    let shown = Held { hands };
    if world.get::<Held>(player) != Some(&shown) {
        commands.entity(player).insert(shown);
    }
}

/// The item worn in each slot of the body.
pub fn worn_slots(equipment: &Equipment) -> impl Iterator<Item = (&EquipmentSlot, &ItemId)> {
    equipment
        .slots
        .iter()
        .filter_map(|(place, item)| match place {
            EquipmentPlace::Slot(slot) => Some((slot, item)),
            EquipmentPlace::Hand(_) | EquipmentPlace::Tool => None,
        })
}

/// The item in each equipped hand.
pub fn hands(equipment: &Equipment) -> BTreeMap<HandIndex, ItemId> {
    equipment
        .slots
        .iter()
        .filter_map(|(slot, &item)| match slot {
            EquipmentPlace::Hand(hand) => Some((*hand, item)),
            EquipmentPlace::Slot(_) | EquipmentPlace::Tool => None,
        })
        .collect()
}

pub fn equip_requested(world: &mut World) {
    player_requests::<EquipRequest>(world, RequestNeeds::Command, |world, player, request| {
        equip(world, player, request.slot as usize, request.hand)
    });
}

pub fn unequip(world: &mut World) {
    player_requests::<UnequipRequest>(world, RequestNeeds::Command, |world, player, request| {
        unequip_slot(
            world,
            player,
            request.slot,
            request.into.map(|into| into as usize),
        )
    });
}

messages! {
    /// What the game says of what the player wears and holds.
    pub struct EquipmentMessages {
        /// What a player is told of the item `{item}` that can't be worn.
        cannot_wear[item: Line],
        /// What a player is told of the item `{item}` they can't wear before they meet `{requirement}`.
        needs[item: Line, requirement: Line],
        /// What a player is told of the item `{item}` whose slot their body lacks.
        body_cannot_wear[item: Line],
        /// What a player is told taking off a slot where nothing is worn.
        nothing_worn,
        /// What a player is told taking off the item `{item}` they don't wear.
        not_worn[item: Line],
        /// The name of the one place on the pane for a hand, when the body has none of its own.
        hand_name,
        /// The name of the tool slot on the pane.
        tool_name,
        /// Where an item is worn, as a pane says it beside the place's name: the kind of `{place}`,
        /// one of `slot`, `tool`, `both_hands`, `any_hand` or `hand`.
        where_held(place: Text),
        /// How a screen reader names a place of the pane holding `{item}`, the place `{place}`.
        worn_as(item: Text, place: Text),
        /// How a screen reader names the empty place `{place}` of the pane.
        empty_named(place: Text),
        /// What an empty worn slot says of itself, beside its name.
        empty_slot_tip,
        /// What the empty tool slot says of itself.
        empty_tool_tip,
        /// What an empty hand says of itself and the stances `{stances}` that land in it.
        empty_hand_landing_tip(stances: Text),
        /// What an empty hand says of itself when no stance is made for it.
        empty_hand_tip,
        /// What the input that takes a worn item off does, as a hint names it.
        take_off,
        /// What the input that carries a worn item away does, as a hint names it.
        carry_off,
        /// What a hand says of a held item whose stance, `{stance}`, the body cannot show.
        unseen_stance(stance: Text),
        /// Where an item is worn, as `where_held` says it, and the stance `{stance}` it brings.
        with_stance(place: Text, stance: Text),
        /// Where an item is worn, as `where_held` says it, when the body cannot show it.
        unseen(place: Text),
        /// What a hand says when its stance has no set on the body.
        no_set,
    }
}

pub fn scripting(vocabulary: &mut Vocabulary) {
    query::scripting(vocabulary);
}
