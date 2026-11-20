pub mod bag_view;
pub mod card;
pub mod deliver;
pub mod effects;
pub mod grid;
pub mod inline;
pub mod plates;
pub mod query;
pub mod rarity;
pub mod render;
pub mod transfer;
pub mod tray;
pub mod widget;

use crate::core::content::ContentProblem;
use crate::core::script::EventDef;
use crate::core::script::OwnerRow;
use crate::core::script::RunnerSpec;
use crate::core::script::Scripts;
use crate::core::script::Value;
use crate::core::script::ValueType;
use crate::core::script::Vocabulary;
use crate::systems::skill::SKILL_RUNNER;
use std::borrow::Cow;

use bevy_app::App;
use bevy_ecs::component::Component;
use bevy_ecs::entity::{Entity, MapEntities};
use bevy_ecs::message::Message;
use bevy_ecs::query::QueryState;
use bevy_ecs::world::World;
use bevy_replication::Replicated;
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use strum::{EnumDiscriminants, IntoStaticStr, VariantArray};

use crate::core::audio::playback::SfxId;
use crate::core::content::ImageRef;
use crate::core::content::SchemaMark;
use crate::core::content::media::ImageSizes;
use crate::core::content::{
    Content, ContentFile, ContentId, Loading, ModuleRule, ModuleSet, NamedContent, Nearby,
    RowCheck, serial,
};
use crate::core::math::{Offset, Pos, Rng};
use crate::core::script::{
    EventKind, Expr, Instances, Owner, RunnerKind, ScriptModule, ScriptedRow,
};
use crate::core::text::LineText;
use crate::core::time::{self, UnixMillis, WallClock};
use crate::core::units::{MeterPos, Meters, Percent, Seconds};
use crate::systems::GameSettings;
use crate::systems::actor::part::ModelPartId;
use crate::systems::actor::skeleton::{
    self, Colours, EquipmentSlot, LocalOffset, LocalTurn, MoveName, SkeletonId,
};
use crate::systems::actor::stance::StanceId;
use crate::systems::area::{self, AreaId, AreaTag, PropId};
use crate::systems::equipment;
use crate::systems::interface::{CursorId, InterfaceMessages};
use crate::systems::item::deliver::Delivered;
use crate::systems::item::rarity::ItemRarityId;
use crate::systems::mention::Mention;
use crate::systems::messages::{self, messages};
use crate::systems::mount::{self, RiderAct};
use crate::systems::movement::{Position, position};
use crate::systems::npc::Npc;
use crate::systems::party;
use crate::systems::player::RequestNeeds;
use crate::systems::player::{ClientId, PlayedBy, player_requests};
use crate::systems::reach::{self, Pursuit, ReachAct, ReachTarget};
use crate::systems::skill::{self, CastAim, SkillId};
use crate::systems::visibility::{self, Presence, seen_by};

messages! {
    /// What the game says of items changing hands and bags.
    pub struct ItemMessages {
        /// What an exchange asking for items a player lacks says: `{items}`, how many more of which item.
        needs_more[items: Line],
        /// What an exchange says that a player's bag has no room for, `{slots}` slots short.
        needs_slots(slots: Number),
        /// What a player is told whose item, `{item}`, can't change hands.
        bound[item: Line],
        /// What a slot says of an item that may not leave the bag for the window items move into.
        bound_slot,
        /// What moves a whole stack between the bag and a window.
        move_stack,
        /// What moves one of a stack between the bag and a window.
        move_one,
        /// What moves as many of a stack as the player says between the bag and a window.
        move_some,
        /// What the player is asked moving some of a stack of `{item}`.
        move_how_many[item: Line],
        /// How many of a stack the player is moving: `{count}` of its `{all}`.
        some_of(all: Number, count: Number),
        /// How items move between the bag and a window, with the inputs `{stack}`, `{one}` and
        /// `{some}` where the player presses them.
        moves_how[one: Line, some: Line, stack: Line],
        /// How a screen reader names a slot of the bag: slot `{slot}` holding `{item}`.
        slot_named(slot: Number, item: Text),
        /// How a screen reader names a slot of the window `{window}` holding `{stack}`.
        slot_placed(window: Text, stack: Text),
        /// How a screen reader names an empty slot of a window items move into.
        empty_slot,
        /// `{count}` of the item `{item}`, as a slot or a line names them.
        counted(count: Number, item: Text),
        /// How many of an item there are, `{count}`, shown beside its icon and name.
        amount(count: Number),
        /// What a slot holds whose whole stack of `{item}` a window has set aside.
        all_set_aside(item: Text),
        /// What a slot holds of `{item}` with `{aside}` more of it set aside in a window.
        some_set_aside(aside: Number, item: Text),
        /// How full a bag is: `{taken}` of its `{slots}` slots taken.
        room(slots: Number, taken: Number),
        /// What the player is asked splitting a stack of `{item}`.
        split_how_many[item: Line],
        /// How a split stack divides: `{count}` in the new stack, `{left}` left in the one split.
        split_says(count: Number, left: Number),
        /// What the player is asked taking some of a stack of `{item}` back out of a window.
        take_back_how_many[item: Line],
        /// What a window's slot says of an item set aside in it that the bag no longer holds.
        no_longer_carried,
        /// What the input that opens an item's card does, as a hint names it.
        inspect,
        /// What the input that splits a stack does, as a hint names it.
        split,
        /// What the input that puts an item into chat does, as a hint names it.
        add_to_chat,
        /// What the input that uses an item does, as a hint names it.
        use_item,
        /// What the input that puts an item on does, as a hint names it.
        wear,
        /// What takes a whole stack back out of a window, as a hint names it.
        take_back_stack,
        /// What takes one of a stack back out of a window, as a hint names it.
        take_back_one,
        /// What takes some of a stack back out of a window, as a hint names it.
        take_back_some,
        /// What the input that puts an item on the bar does, as a hint names it.
        pin_to_bar,
        /// How many of an item the player holds, `{count}`, as its card tallies them.
        held(count: Number),
        /// How an item held in the hand is worn.
        held_in_hand,
        /// How a tool is worn.
        kept_as_tool,
        /// How an item worn in the slot `{slot}` is worn.
        worn_on(slot: Text),
        /// What an item held in the hand is, beside its category.
        hand,
        /// What a tool is, beside its category.
        tool,
        /// The mark of an item a quest needs.
        quest_item,
        /// The heading over the skills an item grants while it is worn.
        grants,
        /// What a card says of an item whose granted skill is a basic attack.
        lends_basic_attack,
        /// How a granted skill is used, the inputs `{carry}` and `{pin}` where they stand.
        grant_hint[carry: Line, pin: Line],
        /// What an item requires of its wearer, `{requirement}`.
        requires[requirement: Line],
        /// The skill `{skill}` an item casts as it is used.
        casts[skill: Line],
        /// What an item does to the stat `{stat}` while it is in the bag: `{by}`, raising it or
        /// lowering it as `{direction}` is `up` or `down`.
        carried_boost_by(by: Number, direction: Text, stat: Text),
        /// What an item does to the stat `{stat}` without a fixed amount while it is in the bag.
        carried_boosts(stat: Text),
        /// What a card says of an item that stacks without limit.
        stacks_without_limit,
        /// How many of an item, `{count}`, stack in one slot.
        stacks_up_to(count: Number),
        /// What an item does to the stat `{stat}` without a fixed amount.
        boosts(stat: Text),
        /// What an item does to the stat `{stat}`: `{by}`, raising it or lowering it as
        /// `{direction}` is `up` or `down`.
        boost_by(by: Number, direction: Text, stat: Text),
        /// What a player is told of `{item}`, which their bag does not hold.
        not_in_bag[item: Line],
        /// What a player is told using an empty slot of the bag.
        nothing_there,
        /// What a player is told whose stack changed in their bag before it could be moved.
        stack_changed,
        /// The heading over the bag `{heading}` with how full it is: `{taken}` of its `{slots}` slots taken.
        heading_room(heading: Text, slots: Number, taken: Number),
        /// The heading over the bag `{heading}` while it is narrowed: `{shown}` of its `{of}` stacks shown.
        heading_shown(heading: Text, shown: Number, of: Number),
        /// What the field that narrows the bag says while nothing is typed in it.
        search_bag,
        /// What clears what narrows the bag.
        clear,
        /// The entry of the bag's type picker that shows every type.
        all_types,
        /// An entry of the bag's type picker: the type `{category}` and how many stacks of it the bag holds, `{count}`.
        category_count(category: Text, count: Number),
        /// What the button that sorts the bag is called.
        sort,
        /// What the button that sorts the bag is called while a sort is on its way.
        sorting,
        /// What the menu of sorts says every sort does.
        sort_says,
        /// The sort that groups the bag by type.
        sort_by_kind,
        /// The sort that puts the rarest first.
        sort_by_rarity,
        /// The sort by name.
        sort_by_name,
        /// What the bag says when nothing in it matches the words `{words}`.
        nothing_matches(words: Text),
        /// What the bag says when nothing of the type `{category}` in it matches the words `{words}`.
        nothing_matches_of_type(words: Text, category: Text),
        /// What the bag says when it holds nothing of the type `{category}`.
        nothing_of_type(category: Text),
        /// What carrying a stack over a narrowed bag says.
        clear_to_arrange,
        /// What letting a carried stack go over an empty slot of the bag does.
        move_here,
        /// What letting a carried stack go over a slot holding `{item}` does.
        swap_with(item: Text),
        /// What letting a carried stack go over a stack of its item does: `{count}` of it join.
        join_some(count: Number),
        /// What letting a carried stack go over a full stack of its item does.
        swap,
        /// What the item card's action that lifts the stack it came from, to move it in the bag, is called.
        lift,
        /// What a lifted stack says over the bag: where it goes.
        place_lifted,
        /// What a player or a script is told moving items for someone who has no bag.
        no_bag,
        /// What a script is told moving no items.
        nothing_to_move,
        /// Whose a dropped item is while it is kept for them: `{name}`'s.
        reserved_for(name: Text),
        /// Whose a dropped item is while it is kept for the party `{name}` is in.
        reserved_for_party(name: Text),
        /// How many more kinds of items, `{count}`, lie where a pile's plate can't name them all.
        more_kinds(count: Number),
        /// The name of each category of items.
        category(category: Text),
        /// The name of each category of items, of many of them.
        category_plural(category: Text),
        /// What a player is told as a script trades items with nobody to trade with.
        nobody_to_trade,
        /// What a player is told as a script uses an item with nobody to use it.
        nobody_to_use,
        /// What a player is told as a script equips an item with nobody to equip.
        nobody_to_equip,
        /// What a player is told as a script unequips an item with nobody to unequip.
        nobody_to_unequip,
        /// What a player is told as a script drops items with nobody to drop them.
        nobody_to_drop,
        /// What a player is told as a script moves items between two with one of them missing.
        nobody_to_trade_between,
    }
}

pub fn register(app: &mut App) {
    use bevy_replication::AppMessageExt;

    use bevy_replication::AppReplicationExt;

    app.replicate_private::<Inventory>()
        .replicate::<DroppedItem>()
        .replicate::<Reservation>()
        .add_client_message::<UseItemRequest>()
        .add_client_message::<DropItemRequest>()
        .add_client_message::<SplitStackRequest>()
        .add_client_message::<MoveStackRequest>()
        .add_client_message::<SortBagRequest>()
        .add_mapped_client_message::<PickupRequest>()
        .add_server_message::<ItemShown>()
        .add_mapped_server_message::<ItemConsumed>()
        .add_mapped_server_message::<ItemsDropped>();
}

/// The bag of the player's character, on a client.
pub fn my_bag(world: &World) -> Option<&Inventory> {
    crate::systems::player::session::my_character(world)?.get::<Inventory>()
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Inventory {
    /// One entry for each slot of the bag, in the order the bag shows them. `None` is an empty
    /// slot, kept where it is so the stacks around it keep their places.
    pub slots: Vec<Option<ItemStack>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ItemStack {
    pub item: ItemId,
    pub count: u32,
}

/// The file form leaves a count of one out; the wire form always carries it, as a binary format
/// cannot tell an omitted field from a missing one.
#[derive(JsonSchema, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ItemStackRow {
    item: ItemId,
    #[schemars(transform = SchemaMark::Count.with(true))]
    #[schemars(extend("default" = serial::one()))]
    #[serde(default = "serial::one", skip_serializing_if = "serial::is_one")]
    count: u32,
}

impl Serialize for ItemStack {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            return ItemStackRow {
                item: self.item,
                count: self.count,
            }
            .serialize(serializer);
        }
        (self.item, self.count).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ItemStack {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<ItemStack, D::Error> {
        if deserializer.is_human_readable() {
            let ItemStackRow { item, count } = ItemStackRow::deserialize(deserializer)?;
            return Ok(ItemStack { item, count });
        }
        let (item, count) = <(ItemId, u32)>::deserialize(deserializer)?;
        Ok(ItemStack { item, count })
    }
}

impl JsonSchema for ItemStack {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("ItemStack")
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        ItemStackRow::json_schema(generator)
    }

    fn inline_schema() -> bool {
        true
    }
}

impl ItemStack {
    pub const fn new(item: ItemId, count: u32) -> ItemStack {
        ItemStack { item, count }
    }

    /// The stack laid in slots as full as a stack of its item holds, the full ones first.
    pub fn in_full_stacks(self, content: &Content) -> impl Iterator<Item = ItemStack> + use<> {
        let max = self.item.get(content).stack_max().max(1);
        let rest = self.count % max;
        std::iter::repeat_n(ItemStack::new(self.item, max), (self.count / max) as usize)
            .chain((rest > 0).then_some(ItemStack::new(self.item, rest)))
    }

    /// The stack in plain words, as `named` says it.
    pub fn describe(self, content: &Content) -> String {
        self.named().plain_words(content)
    }

    /// Whether the item is one that can't change hands.
    pub fn bound(self, content: &Content) -> bool {
        self.item.get(content).bound()
    }

    /// What a player is told whose stack can't change hands, in the game's words.
    pub fn bound_line(item: ItemId, content: &Content) -> LineText {
        messages::of(content)
            .item
            .bound(Mention::Item(item).span(content))
    }

    /// Whether every one of `stacks` can change hands, and if not, what the player is told of
    /// the first that can't.
    pub fn refuse_bound(stacks: &[ItemStack], content: &Content) -> Result<(), LineText> {
        match stacks.iter().find(|stack| stack.bound(content)) {
            Some(bound) => Err(ItemStack::bound_line(bound.item, content)),
            None => Ok(()),
        }
    }

    /// The stack as a line names it: how many, beside the item's icon and name.
    pub fn named(self) -> LineText {
        inline::InlineItem::from(self).into()
    }

    /// `stacks` as a sentence names them, one after another.
    pub fn listed(messages: &InterfaceMessages, stacks: &[ItemStack]) -> LineText {
        messages.listed(stacks.iter().map(|stack| stack.named()).collect())
    }

    /// `stacks` added up, as a cost or a recipe reads them.
    pub fn summed(stacks: &[ItemStack]) -> LineText {
        added(stacks.iter().map(|&stack| inline::InlineItem::from(stack)))
    }

    /// `stacks` added up by their icons alone, as a view whose players know the items by them
    /// shows a price.
    pub fn summed_by_icon(stacks: &[ItemStack]) -> LineText {
        added(
            stacks
                .iter()
                .map(|&stack| inline::InlineItem::from(stack).nameless()),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExchangeRefusal {
    Missing(ItemStack),
    NoRoom { slots: u32 },
}

impl ExchangeRefusal {
    /// What the refusal tells the player: the item it is short of as the interface mentions one.
    pub fn line(self, content: &Content) -> LineText {
        match self {
            ExchangeRefusal::Missing(stack) => messages::of(content).item.needs_more(stack.named()),
            ExchangeRefusal::NoRoom { slots } => {
                messages::of(content).item.needs_slots(slots).into()
            }
        }
    }
}

impl Inventory {
    /// The bag of one who carries none: it holds nothing.
    pub fn empty() -> Inventory {
        Inventory { slots: Vec::new() }
    }

    /// A bag of `size` slots, every one of them empty.
    pub fn sized(size: u32) -> Inventory {
        Inventory {
            slots: vec![None; size as usize],
        }
    }

    /// The bag `owner` carries, or an empty one when they carry none.
    pub fn of(world: &World, owner: impl Into<Option<Entity>>) -> Inventory {
        owner
            .into()
            .and_then(|owner| world.get::<Inventory>(owner))
            .cloned()
            .unwrap_or_else(Inventory::empty)
    }

    /// How many slots the bag has, empty or not.
    pub fn size(&self) -> u32 {
        self.slots.len() as u32
    }

    /// The stack in slot `slot`, when it holds one.
    pub fn at(&self, slot: usize) -> Option<ItemStack> {
        self.slots.get(slot).copied().flatten()
    }

    /// Every stack the bag holds, with the slot it is in, in the bag's order.
    pub fn stacks(&self) -> impl Iterator<Item = (usize, ItemStack)> + '_ {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(slot, stack)| stack.map(|stack| (slot, stack)))
    }

    pub fn count(&self, item: ItemId) -> u32 {
        self.stacks()
            .filter(|(_, stack)| stack.item == item)
            .fold(0, |total, (_, stack)| total.saturating_add(stack.count))
    }

    /// How many of the stack in slot `slot` `stacks` take up, each item's count taken out of
    /// its slots in the bag's order.
    pub fn taken_from(&self, slot: u32, stacks: &[ItemStack]) -> u32 {
        let Some(held) = self.at(slot as usize) else {
            return 0;
        };
        let mut left: u32 = stacks
            .iter()
            .filter(|stack| stack.item == held.item)
            .map(|stack| stack.count)
            .sum();
        for (index, stack) in self.stacks() {
            if stack.item != held.item {
                continue;
            }
            let taken = left.min(stack.count);
            if index == slot as usize {
                return taken;
            }
            left -= taken;
        }
        0
    }

    pub fn slot_of(&self, item: ItemId) -> Option<usize> {
        self.stacks()
            .find(|(_, stack)| stack.item == item)
            .map(|(slot, _)| slot)
    }

    pub fn taken_slots(&self) -> u32 {
        self.stacks().count() as u32
    }

    pub fn free_slots(&self) -> u32 {
        self.size() - self.taken_slots()
    }

    pub fn is_full(&self) -> bool {
        self.free_slots() == 0
    }

    /// Splits `count` off the stack in `slot` into a stack of its own, in the first empty slot
    /// after it, or from the bag's start when none after it is. A split that would leave either
    /// stack empty changes nothing; one the bag has no slot for is refused.
    pub fn split(&mut self, slot: usize, count: u32) -> Result<(), ExchangeRefusal> {
        let whole = match self.at(slot) {
            Some(stack) if (1..stack.count).contains(&count) => stack,
            _ => return Ok(()),
        };
        let size = self.slots.len();
        let Some(into) = (1..size)
            .map(|step| (slot + step) % size)
            .find(|&at| self.slots[at].is_none())
        else {
            return Err(ExchangeRefusal::NoRoom { slots: 1 });
        };
        self.slots[slot] = Some(ItemStack::new(whole.item, whole.count - count));
        self.slots[into] = Some(ItemStack::new(whole.item, count));
        Ok(())
    }

    /// Takes each of `taken` out of the slot it names, leaving a slot it empties empty. Unless
    /// every slot holds what is taken from it, nothing is taken.
    pub fn take_out(&mut self, taken: &[SlotStack]) -> bool {
        let mut slots = self.slots.clone();
        for SlotStack { slot, stack } in taken {
            match slots.get_mut(*slot as usize).and_then(Option::as_mut) {
                Some(held)
                    if held.item == stack.item && (1..=held.count).contains(&stack.count) =>
                {
                    held.count -= stack.count;
                }
                _ => return false,
            }
        }
        self.slots = slots;
        self.clear_spent();
        true
    }

    /// Moves the stack in `from` to the slot `to`, as a drop in the bag does: an empty slot takes
    /// it, a stack of the same item takes as much as it has room for (what doesn't fit stays in
    /// `from`, and the two swap when it has no room at all), and any other stack swaps places
    /// with it. Refused when `from` no longer holds `item`, the item the player moved.
    pub fn move_stack(
        &mut self,
        content: &Content,
        from: usize,
        to: usize,
        item: ItemId,
    ) -> Result<(), StackChanged> {
        let Some(moved) = self.at(from).filter(|stack| stack.item == item) else {
            return Err(StackChanged);
        };
        if from == to || to >= self.slots.len() {
            return Ok(());
        }
        let room = match self.at(to) {
            Some(kept) if kept.item == item => {
                item.get(content).stack_max().saturating_sub(kept.count)
            }
            _ => 0,
        };
        if room == 0 {
            self.slots.swap(from, to);
            return Ok(());
        }
        let joined = moved.count.min(room);
        let kept = self.at(to).map_or(0, |kept| kept.count);
        self.slots[to] = Some(ItemStack::new(item, kept + joined));
        self.slots[from] =
            (moved.count > joined).then(|| ItemStack::new(item, moved.count - joined));
        Ok(())
    }

    /// Sorts the whole bag once by `by`: each item's stacks joined as far as a stack of it holds,
    /// then laid from the first slot on with no gaps between them.
    pub fn sort(&mut self, content: &Content, by: BagSort) {
        let held: Vec<ItemStack> = self.stacks().map(|(_, stack)| stack).collect();
        let mut stacks: Vec<ItemStack> = merged(&held)
            .into_iter()
            .flat_map(|whole| whole.in_full_stacks(content))
            .collect();
        let names = messages::of(content).locale.collator();
        stacks.sort_by(|a, b| by.order(content, &names, *a, *b));
        let size = self.slots.len();
        self.slots = stacks
            .into_iter()
            .map(Some)
            .chain(std::iter::repeat(None))
            .take(size)
            .collect();
    }

    /// Holds `count` of `item`, however many it held before, laid as new items are, as far as
    /// the bag has room.
    pub fn set_count(&mut self, content: &Content, item: ItemId, count: u32) {
        for slot in &mut self.slots {
            if slot.is_some_and(|stack| stack.item == item) {
                *slot = None;
            }
        }
        self.add(content, item, count);
    }

    pub fn exchange(
        &mut self,
        content: &Content,
        takes: &[ItemStack],
        gives: &[ItemStack],
    ) -> Result<(), ExchangeRefusal> {
        let mut next = self.clone();
        for take in merged(takes) {
            let held = next.count(take.item);
            if held < take.count {
                return Err(ExchangeRefusal::Missing(ItemStack::new(
                    take.item,
                    take.count - held,
                )));
            }
            next.remove(take);
        }
        let left: Vec<ItemStack> = gives
            .iter()
            .map(|give| ItemStack::new(give.item, next.add(content, give.item, give.count)))
            .filter(|left| left.count > 0)
            .collect();
        let slots: u32 = merged(&left)
            .into_iter()
            .map(|left| left.in_full_stacks(content).count() as u32)
            .sum();
        if slots > 0 {
            return Err(ExchangeRefusal::NoRoom { slots });
        }
        *self = next;
        Ok(())
    }

    /// Gives each of `stacks` that fits, tried on its own and in order, so one that finds no
    /// room never holds back the others.
    pub fn give_what_fits(&mut self, content: &Content, stacks: &[ItemStack]) -> Delivered {
        let mut delivered = Delivered::default();
        for &stack in stacks {
            match self.exchange(content, &[], &[stack]) {
                Ok(()) => delivered.kept.push(stack),
                Err(_) => delivered.sent.push(stack),
            }
        }
        delivered
    }

    pub fn take_up_to(&mut self, stacks: &[ItemStack]) {
        for stack in stacks {
            self.remove(*stack);
        }
    }

    /// Puts `count` of `item` in the bag: onto its stacks first, as far as they have room, then
    /// into the empty slots from the first on. How many found no room.
    fn add(&mut self, content: &Content, item: ItemId, mut count: u32) -> u32 {
        let stack_max = item.get(content).stack_max();
        for stack in self.slots.iter_mut().flatten() {
            if count == 0 {
                break;
            }
            if stack.item != item {
                continue;
            }
            let take = stack_max.saturating_sub(stack.count).min(count);
            stack.count += take;
            count -= take;
        }
        for slot in self.slots.iter_mut().filter(|slot| slot.is_none()) {
            if count == 0 {
                break;
            }
            let take = stack_max.min(count);
            *slot = Some(ItemStack::new(item, take));
            count -= take;
        }
        count
    }

    /// Takes as many of `stack` as the bag holds, from its last stacks back.
    fn remove(&mut self, stack: ItemStack) {
        let mut left = stack.count;
        for held in self
            .slots
            .iter_mut()
            .rev()
            .flatten()
            .filter(|held| held.item == stack.item)
        {
            let take = held.count.min(left);
            held.count -= take;
            left -= take;
        }
        self.clear_spent();
    }

    fn clear_spent(&mut self) {
        for slot in &mut self.slots {
            if slot.is_some_and(|stack| stack.count == 0) {
                *slot = None;
            }
        }
    }
}

/// Why a stack wasn't moved: the slot it was moved from no longer held it, the bag having
/// changed since the player saw it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StackChanged;

impl StackChanged {
    pub fn line(self, content: &Content) -> LineText {
        messages::of(content).item.stack_changed().into()
    }
}

/// An order a whole bag is sorted in, each breaking its ties on the next thing it names, then
/// on the larger stack first.
#[derive(
    JsonSchema,
    Serialize,
    Deserialize,
    IntoStaticStr,
    VariantArray,
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
)]
#[serde(rename_all = "snake_case")]
pub enum BagSort {
    /// Category, then rarity from rarest, then name.
    Kind,
    /// Rarity from rarest, then category, then name.
    Rarity,
    /// Name, as the game's locale orders words.
    Name,
}

impl BagSort {
    fn order(
        self,
        content: &Content,
        names: &icu_collator::CollatorBorrowed<'static>,
        a: ItemStack,
        b: ItemStack,
    ) -> std::cmp::Ordering {
        let (one, other) = (a.item.get(content), b.item.get(content));
        let category = || one.category().cmp(&other.category());
        let rarity = || {
            let rank = |def: &ItemDef| def.rarity.get(content).rank;
            rank(other).cmp(&rank(one))
        };
        let name = || names.compare(&one.name, &other.name);
        let by = match self {
            BagSort::Kind => category().then_with(rarity).then_with(name),
            BagSort::Rarity => rarity().then_with(category).then_with(name),
            BagSort::Name => name(),
        };
        by.then(a.item.cmp(&b.item)).then(b.count.cmp(&a.count))
    }
}

pub fn merged(stacks: &[ItemStack]) -> Vec<ItemStack> {
    let mut merged: Vec<ItemStack> = Vec::new();
    for stack in stacks {
        match merged
            .iter_mut()
            .find(|existing| existing.item == stack.item)
        {
            Some(existing) => existing.count = existing.count.saturating_add(stack.count),
            None => merged.push(*stack),
        }
    }
    merged
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Reservation {
    pub by: Option<ClientId>,
    pub at: Seconds,
}

impl Reservation {
    /// Who holds the claim at `now`, a claim lasting `lasts`.
    pub fn holder(&self, now: Seconds, lasts: Seconds) -> Option<ClientId> {
        self.by.filter(|_| now - self.at < lasts)
    }

    /// Whether `taker` may take what is reserved so: its holder, and whoever plays in a party
    /// with them. What is reserved for nobody is anyone's.
    pub fn allows(&self, taker: &DropTaker) -> bool {
        self.by
            .is_none_or(|by| taker.client == Some(by) || taker.party.contains(&by))
    }
}

/// Who would take a drop: the client that plays a character, and the clients playing the
/// characters of its party, whose drops are open to it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DropTaker {
    pub client: Option<ClientId>,
    pub party: Vec<ClientId>,
}

impl DropTaker {
    pub fn of(world: &World, character: Entity) -> DropTaker {
        DropTaker {
            client: world.get::<PlayedBy>(character).map(|by| by.client),
            party: party::companions(world, character),
        }
    }

    /// The player of this client.
    pub fn me(world: &World) -> DropTaker {
        let me = crate::systems::player::session::me(world);
        DropTaker {
            client: crate::systems::player::session::my_id(world),
            party: me.map_or_else(Vec::new, |me| party::companions(world, me)),
        }
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DroppedItem {
    pub item: ItemId,
    pub count: u32,
    /// When it will be gone, by the server's clock.
    pub until: UnixMillis,
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct UseItemRequest {
    pub slot: u32,
}

/// Tells a client to show its player the card of an item.
#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ItemShown {
    pub item: ItemId,
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DropItemRequest {
    pub slot: u32,
}

/// Some of what a slot of the bag holds: `stack.count` of the stack in `slot`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SlotStack {
    pub slot: u32,
    pub stack: ItemStack,
}

/// Splits `count` off the stack in the bag's slot `slot`, into a stack of its own in the next
/// empty slot.
#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SplitStackRequest {
    pub slot: u32,
    pub count: u32,
}

/// Moves the stack in the bag's slot `from` to its slot `to`, as `Inventory::move_stack` does.
/// `item` is what the player saw in `from`: the move is refused when it holds something else.
#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MoveStackRequest {
    pub from: u32,
    pub to: u32,
    pub item: ItemId,
}

/// Sorts the whole bag once, in the order `by`.
#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SortBagRequest {
    pub by: BagSort,
}

#[derive(Message, Serialize, Deserialize, MapEntities, Clone, Debug, PartialEq)]
pub struct PickupRequest {
    #[entities]
    pub target: Entity,
}

#[derive(Message, Serialize, Deserialize, MapEntities, Clone, Debug, PartialEq)]
pub struct ItemConsumed {
    pub item: ItemId,
    #[entities]
    pub actor: Entity,
}

#[derive(Message, Serialize, Deserialize, MapEntities, Clone, Debug, PartialEq)]
pub struct ItemsDropped {
    #[entities]
    pub items: Vec<Entity>,
    pub from: Pos<Meters>,
}

pub type ItemId = ContentId<ItemDef>;

/// The pixels of an item's icon: as many as a slot draws it in, so it shows there pixel for pixel.
pub const ICON_SIZE: (u32, u32) = (
    ui::controls::SLOT_ICON as u32,
    ui::controls::SLOT_ICON as u32,
);
/// The pixels of an item's card art, painted for the item alone.
pub const CARD_SIZE: (u32, u32) = (256, 180);

/// A thing a character carries: its three pictures (the prop it is lying on the ground, the icon it
/// is in a slot, the art on its card), its sounds, what kind of thing it is and what it does.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct ItemDef {
    pub name: String,
    pub rarity: ItemRarityId,
    pub look: PropId,
    pub icon: ImageRef,
    pub card: ImageRef,
    pub sfx: ItemSfx,
    pub kind: ItemKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modules: Vec<ItemModule>,
}

impl ContentFile for ItemDef {
    const TABLE: &'static str = "item";

    fn fresh(near: &Nearby) -> Result<ItemDef, String> {
        let sound = near.file()?;
        Ok(ItemDef {
            name: near.name(),
            rarity: near.file()?,
            look: near.file()?,
            icon: near.image_sized(ICON_SIZE)?,
            card: near.image_sized(CARD_SIZE)?,
            sfx: ItemSfx {
                pickup: sound,
                trade: sound,
                drop: sound,
                on_use: None,
            },
            kind: ItemKind::Resource,
            modules: Vec::new(),
        })
    }
}

/// Every item's icon and card art are the size their slots and cards draw.
/// Checks that every usable item that casts no skill has a script answering its use.
pub fn check_used(loading: &mut Loading) {
    let content = loading.content();
    let scripts = content.built::<Scripts>();
    for (id, def) in loading.rows::<ItemDef>(&content) {
        let usable = matches!(def.kind, ItemKind::Usable { casts: None });
        let answers = scripts
            .may_do(OwnerRow::of_row(id))
            .answers(EventKind::of::<Used>());
        if usable && !answers {
            loading.row_error(
                id,
                "/modules",
                ContentProblem::new(
                    "usable without used",
                    format!("item/{}", id.path(&content)),
                    "a script answering used on every usable item that casts no skill",
                ),
            );
        }
    }
}

pub fn check_pictures(loading: &mut Loading) {
    let content = loading.content();
    let sizes = content.built::<ImageSizes>();
    for (id, item) in loading.rows::<ItemDef>(&content) {
        let mut check = RowCheck::default();
        for (field, picture, wanted) in [
            ("/icon", &item.icon, ICON_SIZE),
            ("/card", &item.card, CARD_SIZE),
        ] {
            if let Some(&size) = sizes.0.get(picture.as_str())
                && size != wanted
            {
                check.error(
                    field,
                    "wrong size",
                    format!("{} × {} pixels", size.0, size.1),
                    format!("{} × {} pixels", wanted.0, wanted.1),
                );
            }
        }
        loading.row_errors(id, check);
    }
}

/// Every look of a worn item is on a skeleton that has the item's slot, one look a skeleton, and
/// lays parts made on that skeleton in colours of its tints.
pub fn check_looks(loading: &mut Loading) {
    let content = loading.content();
    for (id, item) in loading.rows::<ItemDef>(&content) {
        let Some(Wear::Worn { slot, looks }) = item.wear() else {
            continue;
        };
        let mut check = RowCheck::default();
        let mut skeletons = std::collections::BTreeSet::new();
        for (index, look) in looks.iter().enumerate() {
            let at = format!("/kind/looks/{index}");
            let skeleton_path = look.skeleton.path(&content);
            if !skeletons.insert(look.skeleton) {
                check.error(
                    format!("{at}/skeleton"),
                    "skeleton twice",
                    skeleton_path,
                    "one look a skeleton",
                );
            }
            let slots = &look.skeleton.get(&content).slots;
            if !slots.contains(slot) {
                check.error(
                    format!("{at}/skeleton"),
                    "no such slot",
                    format!("{skeleton_path} has no slot {slot}"),
                    crate::systems::actor::model::expected_name("slots", slots, skeleton_path),
                );
            }
            for (part_index, part) in look.parts.iter().enumerate() {
                let made_on = part.get(&content).skeleton;
                if made_on != look.skeleton {
                    check.error(
                        format!("{at}/parts/{part_index}"),
                        "part of another skeleton",
                        format!(
                            "{} is made on {}",
                            part.path(&content),
                            made_on.path(&content)
                        ),
                        format!("a part made on {skeleton_path}"),
                    );
                }
            }
            crate::systems::actor::model::check_meshed(
                &content,
                &look.parts,
                &format!("{at}/parts"),
                &mut check,
            );
            crate::systems::actor::material::check_colours(
                &content,
                look.skeleton,
                &look.colours,
                &format!("{at}/colours"),
                &mut check,
            );
        }
        loading.row_errors(id, check);
    }
}

impl NamedContent for ItemDef {
    fn shown_name(&self) -> &str {
        &self.name
    }
}

impl ModuleSet for ItemDef {
    type Module = ItemModule;

    fn modules(&self) -> &[ItemModule] {
        &self.modules
    }

    fn modules_mut(&mut self) -> &mut Vec<ItemModule> {
        &mut self.modules
    }

    fn rule(_: &ItemModule) -> ModuleRule {
        ModuleRule::AtMostOne
    }
}

impl ScriptedRow for ItemDef {
    const OWNER: RunnerKind = ITEM_RUNNER;

    fn script(module: &ItemModule) -> Option<&ScriptModule> {
        match module {
            ItemModule::Script(script) => Some(script),
            _ => None,
        }
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize, EnumDiscriminants)]
#[serde(tag = "type", rename_all = "snake_case")]
#[strum_discriminants(name(ItemModuleKind))]
pub enum ItemModule {
    Flavor(Flavor),
    Stack(StackLimit),
    QuestItem,
    Bound,
    Currency,
    Script(ScriptModule),
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct Flavor {
    pub text: String,
}

#[derive(JsonSchema, Clone, Copy, Serialize, Deserialize)]
pub struct StackLimit {
    pub max: u32,
}

#[derive(
    JsonSchema,
    Serialize,
    Deserialize,
    IntoStaticStr,
    VariantArray,
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
)]
#[serde(rename_all = "snake_case")]
pub enum ItemCategory {
    Consumable,
    Equipment,
    Material,
    Currency,
}

impl ItemCategory {
    /// Its name, as the game's messages give it.
    pub fn named(self, messages: &ItemMessages) -> String {
        messages.category(<&'static str>::from(self))
    }
}

impl ItemDef {
    pub fn flavor(&self) -> Option<&str> {
        self.modules.iter().find_map(|module| match module {
            ItemModule::Flavor(flavor) => Some(flavor.text.as_str()),
            _ => None,
        })
    }

    pub fn stack_max(&self) -> u32 {
        self.modules
            .iter()
            .find_map(|module| match module {
                ItemModule::Stack(stack) => Some(stack.max),
                _ => None,
            })
            .unwrap_or(1)
    }

    pub fn has(&self, kind: ItemModuleKind) -> bool {
        self.modules
            .iter()
            .any(|module| ItemModuleKind::from(module) == kind)
    }

    pub fn bound(&self) -> bool {
        self.has(ItemModuleKind::Bound) || self.has(ItemModuleKind::QuestItem)
    }

    /// Where the item is worn, when it is equipment.
    pub fn wear(&self) -> Option<&Wear> {
        match &self.kind {
            ItemKind::Equipment { wear, .. } => Some(wear),
            _ => None,
        }
    }

    pub fn category(&self) -> ItemCategory {
        if self.has(ItemModuleKind::Currency) {
            return ItemCategory::Currency;
        }
        match self.kind {
            ItemKind::Usable { .. } => ItemCategory::Consumable,
            ItemKind::Equipment { .. } => ItemCategory::Equipment,
            ItemKind::Resource => ItemCategory::Material,
        }
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ItemKind {
    Equipment {
        #[serde(flatten)]
        wear: Wear,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        requires: Option<Expr>,
        #[schemars(transform = SchemaMark::Usage.with("known while worn"))]
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        grants: Vec<SkillId>,
    },
    Usable {
        /// The skill a use of it casts, aimed as the skill is, by whoever carries the item,
        /// learned or not. What the cast takes is the skill's cost: the item itself, when it is
        /// used up. Without one, a use raises `used` on the item's own script.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        casts: Option<SkillId>,
    },
    Resource,
}

/// How an item is worn: held in a hand, putting its wearer in a stance and hanging from the hand by
/// its grip, worn in one of the body's slots, laying the parts of its look on the wearer's
/// skeleton over the body, or kept in the tool slot. A worn item with no look on the wearer's
/// skeleton counts all the same, and is never drawn.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "wear", rename_all = "snake_case")]
pub enum Wear {
    Held {
        stance: StanceId,
        #[serde(default, skip_serializing_if = "Grip::is_identity")]
        grip: Grip,
    },
    Worn {
        slot: EquipmentSlot,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        looks: Vec<WornLook>,
    },
    /// Kept in the tool slot, and in the hand its stance names only while its wearer works with it.
    Tool {
        tool: ToolTypeId,
        stance: StanceId,
        #[serde(default, skip_serializing_if = "Grip::is_identity")]
        grip: Grip,
    },
}

/// The parts a worn item lays over a body of `skeleton`, and the colours they are painted in over
/// the wearer's.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WornLook {
    pub skeleton: SkeletonId,
    pub parts: Vec<ModelPartId>,
    #[serde(default, skip_serializing_if = "Colours::is_empty")]
    pub colours: Colours,
}

/// How a prop sits in the hand it hangs from, in the hand's grip frame (see `Hand`): none for a prop
/// whose handle is at its origin with its length along +Y, its face, where it has one, toward +Z, and
/// the edge that leads a blow toward +X.
#[derive(JsonSchema, Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grip {
    #[serde(default, skip_serializing_if = "LocalOffset::is_zero")]
    pub offset: LocalOffset,
    #[serde(default, skip_serializing_if = "LocalTurn::is_zero")]
    pub turn: LocalTurn,
    /// The axis of its own frame the prop is its own mirror image across, so a mirrored hand holds
    /// it turned over that way rather than inside out.
    #[serde(default, skip_serializing_if = "PropAxis::is_z")]
    pub symmetry: PropAxis,
    #[serde(default, skip_serializing_if = "PropHold::is_gripped")]
    pub hold: PropHold,
}

/// How the hand that holds a prop carries it.
#[derive(
    JsonSchema,
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    VariantArray,
    IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum PropHold {
    /// In the fist, turning with the hand: a sword.
    #[default]
    Gripped,
    /// Hanging plumb from the hand, by the point of it the grip puts there: a lantern from its
    /// bail.
    Plumb,
    /// Strapped to the arm the hand is on, the hand held as its strap poses it, so the prop turns
    /// with the arm alone: a shield. In the fist of a hand that has no strap.
    Strapped,
}

/// An axis of a prop's own frame.
#[derive(
    JsonSchema,
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    VariantArray,
    IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum PropAxis {
    X,
    Y,
    #[default]
    Z,
}

impl Wear {
    /// What it is worn as: in a hand, as a tool, or on its slot by the slot's name.
    pub fn named<'w>(&'w self, messages: &'w ItemMessages) -> &'w str {
        match self {
            Wear::Held { .. } => messages.hand(),
            Wear::Worn { slot, .. } => slot.as_str(),
            Wear::Tool { .. } => messages.tool(),
        }
    }

    /// The stance a hand showing the item puts its wearer in and how the item sits in that hand,
    /// when a hand can show it.
    pub fn grasp(&self) -> Option<(StanceId, &Grip)> {
        match self {
            Wear::Held { stance, grip } | Wear::Tool { stance, grip, .. } => Some((*stance, grip)),
            Wear::Worn { .. } => None,
        }
    }

    /// The kind of tool the item is, when it is one.
    pub fn tool_type(&self) -> Option<ToolTypeId> {
        match self {
            Wear::Tool { tool, .. } => Some(*tool),
            Wear::Held { .. } | Wear::Worn { .. } => None,
        }
    }
}

impl Grip {
    pub fn is_identity(&self) -> bool {
        self.offset.is_zero()
            && self.turn.is_zero()
            && self.symmetry.is_z()
            && self.hold.is_gripped()
    }
}

impl PropAxis {
    pub fn is_z(&self) -> bool {
        *self == PropAxis::Z
    }
}

impl PropHold {
    pub fn is_gripped(&self) -> bool {
        *self == PropHold::Gripped
    }
}

/// One line of a drop table: `amount` of `item`, every time or `chance` of the times.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DropRow {
    pub item: ItemId,
    #[schemars(transform = SchemaMark::Count.with(true))]
    #[schemars(extend("default" = serial::one()))]
    #[serde(default = "serial::one", skip_serializing_if = "serial::is_one")]
    pub amount: u32,
    #[schemars(transform = SchemaMark::Blank.with("always"))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chance: Option<Percent>,
}

impl DropRow {
    /// A row that gives `stack` every time.
    pub fn certain(stack: ItemStack) -> DropRow {
        DropRow {
            item: stack.item,
            amount: stack.count,
            chance: None,
        }
    }

    pub fn of(item: ItemId) -> DropRow {
        DropRow {
            item,
            amount: 1,
            chance: None,
        }
    }

    /// What the row drops this time: its whole amount or nothing.
    pub fn rolled(&self, rng: &mut Rng) -> Option<ItemStack> {
        self.chance
            .is_none_or(|chance| chance.rolled(rng))
            .then_some(self.whole())
    }

    /// What the row drops when it does.
    pub fn whole(&self) -> ItemStack {
        ItemStack::new(self.item, self.amount)
    }
}

pub type ToolTypeId = ContentId<ToolTypeDef>;

/// A way of working resources, by the name players are told: what a resource asks for, what the
/// items worn in the tool slot are a kind of, and how the work looks and sounds whatever is worked,
/// so many resources share few kinds.
#[derive(JsonSchema, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolTypeDef {
    pub name: String,
    /// Done with bare hands: no item is of the kind, and a resource asking for it asks for nothing
    /// worn.
    #[serde(default, skip_serializing_if = "serial::is_false")]
    pub by_hand: bool,
    /// The move its worker makes over and over at the work.
    pub work: MoveName,
    /// The cursor over a resource worked this way.
    pub cursor: CursorId,
    pub sfx: ToolTypeSfx,
}

/// The sounds of work of a kind. An extraction has none of its own: what it pays out sounds as any
/// item gained does.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolTypeSfx {
    /// Heard as each move of the work lands, in place of the sounds of the move's clip.
    pub swing: SfxId,
    /// Heard as the last extraction is taken.
    pub ran_out: SfxId,
}

impl ContentFile for ToolTypeDef {
    const TABLE: &'static str = "tool_type";

    fn fresh(near: &Nearby) -> Result<ToolTypeDef, String> {
        let sound = near.file()?;
        Ok(ToolTypeDef {
            name: near.name(),
            by_hand: false,
            work: skeleton::nearest_move(near)?,
            cursor: near.file()?,
            sfx: ToolTypeSfx {
                swing: sound,
                ran_out: sound,
            },
        })
    }

    fn check(&self, check: &mut RowCheck) {
        check.text("/name", &self.name, 1, 64);
    }
}

impl NamedContent for ToolTypeDef {
    fn shown_name(&self) -> &str {
        &self.name
    }
}

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct ItemSfx {
    pub pickup: SfxId,
    pub trade: SfxId,
    pub drop: SfxId,
    #[serde(rename = "use", default, skip_serializing_if = "Option::is_none")]
    pub on_use: Option<SfxId>,
}

pub fn use_item(world: &mut World) {
    player_requests::<UseItemRequest>(world, RequestNeeds::Command, |world, player, request| {
        let slot = request.slot as usize;
        let content = world.resource::<Content>().clone();
        let leaves = world
            .get::<Inventory>(player)
            .and_then(|bag| bag.at(slot))
            .and_then(|stack| skill::cast_by(&content, stack.item))
            .is_some_and(|skill| mount::left_by(world, player, skill));
        if !leaves {
            mount::seat::act(world, player, RiderAct::Items)?;
        }
        use_slot(world, player, slot)
    });
}

pub fn use_held(world: &mut World, actor: Entity, item: ItemId) -> Result<(), LineText> {
    let slot = bag_slot(world, actor, item)?;
    use_slot(world, actor, slot)
}

pub fn bag_slot(world: &World, actor: Entity, item: ItemId) -> Result<usize, LineText> {
    world
        .get::<Inventory>(actor)
        .and_then(|inventory| inventory.slot_of(item))
        .ok_or_else(|| {
            let content = world.resource::<Content>();
            messages::of(content)
                .item
                .not_in_bag(Mention::Item(item).span(content))
        })
}

fn use_slot(world: &mut World, actor: Entity, slot: usize) -> Result<(), LineText> {
    let content = world.resource::<Content>().clone();
    let item = world
        .get::<Inventory>(actor)
        .and_then(|inventory| inventory.at(slot).map(|stack| stack.item))
        .ok_or_else(|| LineText::from(messages::of(&content).item.nothing_there()))?;
    match item.get(&content).kind {
        ItemKind::Equipment { .. } => equipment::equip(world, actor, slot, None),
        ItemKind::Usable { casts: Some(skill) } => {
            skill::cast(world, actor, skill, CastAim::None, skill::CastWay::Once)
        }
        ItemKind::Usable { casts: None } => {
            announce_consumed(world, actor, item);
            raise(world, item, EventKind::of::<Used>(), actor);
            Ok(())
        }
        ItemKind::Resource => Ok(()),
    }
}

pub fn raise(world: &mut World, item: ItemId, kind: EventKind, actor: Entity) {
    world.resource_mut::<Instances>().raise(
        Owner::Row(ITEM_RUNNER, item.raw()),
        kind,
        Some(actor),
        Vec::new(),
        |_| true,
    );
}

pub fn drop_item(world: &mut World) {
    let content = world.resource::<Content>().clone();
    player_requests::<DropItemRequest>(world, RequestNeeds::Command, |world, player, request| {
        let slot = request.slot as usize;
        let removed = match world.get_mut::<Inventory>(player) {
            Some(mut inventory)
                if inventory
                    .at(slot)
                    .is_some_and(|stack| !stack.bound(&content)) =>
            {
                inventory.slots[slot].take()
            }
            _ => None,
        };
        if let Some(stack) = removed {
            scatter_drop(world, player, &[(stack.item, stack.count)], None, None);
        }
        Ok(())
    });
}

/// Splits, moves and sorts the stacks of their bag as each player asks. A bag is its
/// character's to arrange whatever else they are doing, so nothing holds these back.
pub fn arrange_stacks(world: &mut World) {
    let content = world.resource::<Content>().clone();
    player_requests::<SplitStackRequest>(world, RequestNeeds::Nothing, |world, player, request| {
        match world.get_mut::<Inventory>(player) {
            Some(mut inventory) => inventory
                .split(request.slot as usize, request.count)
                .map_err(|refusal| refusal.line(&content)),
            None => Ok(()),
        }
    });
    player_requests::<MoveStackRequest>(world, RequestNeeds::Nothing, |world, player, request| {
        match world.get_mut::<Inventory>(player) {
            Some(mut inventory) => inventory
                .move_stack(
                    &content,
                    request.from as usize,
                    request.to as usize,
                    request.item,
                )
                .map_err(|changed| changed.line(&content)),
            None => Ok(()),
        }
    });
    player_requests::<SortBagRequest>(world, RequestNeeds::Nothing, |world, player, request| {
        if let Some(mut inventory) = world.get_mut::<Inventory>(player) {
            inventory.sort(&content, request.by);
        }
        Ok(())
    });
}

pub fn pickup_request(world: &mut World) {
    player_requests::<PickupRequest>(
        world,
        RequestNeeds::Seat(RiderAct::PickUp),
        |world, player, request| {
            let target = request.target;
            if world.get::<DroppedItem>(target).is_some()
                && visibility::in_sight(world, player, target)
            {
                reach::intend(world, player, ReachTarget::Actor(target), ReachAct::Pickup);
            }
            Ok(())
        },
    );
}

/// The item nearest `point` within a click's reach of it that the player may take.
pub fn pickable_at(world: &mut World, point: Pos<Meters>) -> Option<Entity> {
    let me = DropTaker::me(world);
    dropped_at(world, point, |reservation| takeable(reservation, &me))
}

/// The item nearest `point` within a click's reach of it, whoever may take it.
pub fn lying_at(world: &mut World, point: Pos<Meters>) -> Option<Entity> {
    dropped_at(world, point, |_| true)
}

fn dropped_at(
    world: &mut World,
    point: Pos<Meters>,
    wanted: impl Fn(Option<&Reservation>) -> bool,
) -> Option<Entity> {
    let reach = world.resource::<GameSettings>().drops.click_reach;
    world
        .query::<(Entity, &Position, &DroppedItem, Option<&Reservation>)>()
        .iter(world)
        .filter(|(_, at, _, reservation)| at.pos.distance(point) <= reach && wanted(*reservation))
        .min_by(|a, b| {
            a.1.pos
                .distance(point)
                .0
                .total_cmp(&b.1.pos.distance(point).0)
        })
        .map(|(entity, ..)| entity)
}

/// Whether `taker` may take a dropped item reserved as `reservation` says: anyone may take
/// what is reserved for nobody.
pub fn takeable(reservation: Option<&Reservation>, taker: &DropTaker) -> bool {
    reservation.is_none_or(|reservation| reservation.allows(taker))
}

pub fn pickups(world: &mut World, intents: &mut reach::Intents) {
    for (player, target) in reach::intending(world, intents, ReachAct::Pickup) {
        let reach = world.resource::<GameSettings>().drops.pickup_reach;
        match reach::pursue(world, player, reach) {
            Pursuit::Approaching => {}
            Pursuit::Lost => reach::forget(world, player),
            Pursuit::Arrived => {
                collect(world, player, target);
                reach::forget(world, player);
            }
        }
    }
}

pub fn expire_drops(world: &mut World, drops: &mut QueryState<(Entity, &DroppedItem)>) {
    let now = world.resource::<WallClock>().now;
    let stale: Vec<Entity> = drops
        .iter(world)
        .filter(|(_, dropped)| dropped.until <= now)
        .map(|(entity, _)| entity)
        .collect();
    for entity in stale {
        world.entity_mut(entity).despawn();
    }
}

pub fn reserve(world: &mut World, npc: Entity, attacker: Entity, now: Seconds) {
    let Some(account) = world.get::<PlayedBy>(attacker).map(|owner| owner.client) else {
        return;
    };
    if world.get::<Npc>(npc).is_none() {
        return;
    }
    let claim = world.get::<Reservation>(npc).is_none_or(|reservation| {
        reservation
            .holder(now, world.resource::<GameSettings>().claims_last)
            .is_none_or(|by| by == account)
    });
    if claim {
        world.entity_mut(npc).insert(Reservation {
            by: Some(account),
            at: now,
        });
    }
}

pub fn scatter_drop(
    world: &mut World,
    source: Entity,
    drops: &[(ItemId, u32)],
    reserved_by: Option<ClientId>,
    presence: Option<Presence>,
) {
    let Some(from) = position(world, source) else {
        return;
    };
    let Some(area_id) = world.get::<AreaTag>(source).map(|tag| tag.area) else {
        return;
    };
    scatter_at(world, from, area_id, drops, reserved_by, presence, None);
}

pub fn scatter_at(
    world: &mut World,
    from: Pos<Meters>,
    area_id: AreaId,
    drops: &[(ItemId, u32)],
    reserved_by: Option<ClientId>,
    presence: Option<Presence>,
    life: Option<Seconds>,
) {
    if drops.is_empty() {
        return;
    }
    let content = world.resource::<Content>().clone();
    let area = area::load(&content, area_id);
    let settings = content.built::<GameSettings>().drops;
    let scatter = settings.scatter;
    let now = time::uptime(world);
    let until = world
        .resource::<WallClock>()
        .now
        .after(life.unwrap_or(settings.last));
    let count = drops.len();
    let turn = Rng::fork_of(world).rand_float() * std::f32::consts::TAU;
    let mut items = Vec::with_capacity(count);
    for (index, &(item, stack)) in drops.iter().enumerate() {
        let pos = scatter_pos(from, scatter, turn, index, count, area);
        let entity = world
            .spawn((
                Replicated,
                Position::at(pos),
                AreaTag { area: area_id },
                DroppedItem {
                    item,
                    count: stack,
                    until,
                },
                Reservation {
                    by: reserved_by,
                    at: now,
                },
            ))
            .id();
        if let Some(presence) = &presence {
            world.entity_mut(entity).insert(presence.clone());
        }
        items.push(entity);
    }
    for client in seen_by(world, items[0]) {
        visibility::send_to(
            world,
            client,
            ItemsDropped {
                items: items.clone(),
                from,
            },
        );
    }
}

fn announce_consumed(world: &mut World, actor: Entity, item: ItemId) {
    for client in seen_by(world, actor) {
        visibility::send_to(world, client, ItemConsumed { item, actor });
    }
}

fn collect(world: &mut World, player: Entity, item: Entity) {
    let Some(drop) = world.get::<DroppedItem>(item).cloned() else {
        return;
    };
    let taker = DropTaker::of(world, player);
    if !takeable(world.get::<Reservation>(item), &taker) {
        return;
    }
    let content = world.resource::<Content>().clone();
    let collected = world
        .get_mut::<Inventory>(player)
        .is_some_and(|mut inventory| {
            inventory
                .exchange(&content, &[], &[ItemStack::new(drop.item, drop.count)])
                .is_ok()
        });
    if collected {
        world.entity_mut(item).despawn();
        raise(world, drop.item, EventKind::of::<PickedUp>(), player);
    }
}

/// Where the drop `index` of `count` lands: each as far from `from` as drops scatter, evenly
/// around it from the angle `turn`, on the ground nearest that.
fn scatter_pos(
    from: Pos<Meters>,
    scatter: Meters,
    turn: f32,
    index: usize,
    count: usize,
    area: &area::Area,
) -> Pos<Meters> {
    let angle = turn + std::f32::consts::TAU * index as f32 / count as f32;
    let spread = from + Offset::new(angle.cos() * scatter.0, angle.sin() * scatter.0);
    area.mesh().nearest(spread).unwrap_or(from)
}

/// `items` one after another, with a plus between each two.
fn added(items: impl Iterator<Item = inline::InlineItem>) -> LineText {
    items
        .enumerate()
        .fold(LineText::default(), |line, (place, item)| {
            line.then(if place == 0 { "" } else { " + " }).then(item)
        })
}

pub const ITEM_RUNNER: RunnerKind = RunnerKind::new("item");

/// What a use hands its script: the aimed actor, the aimed point and the rank used at. An item
/// raises it with none of them.
const USED_FIELDS: &[(&str, ValueType)] = &[
    ("target", ValueType::Actor),
    ("point", ValueType::Point),
    ("rank", ValueType::Integer),
];

/// The aimed actor, the aimed point and the rank an item or a skill was used at, in the order
/// the event hands them to its script.
pub fn used_payload(target: Option<Entity>, point: Option<Pos<Meters>>, rank: i64) -> Vec<Value> {
    vec![Value::Actor(target), Value::Point(point), Value::Int(rank)]
}

/// Where a script finds what a use hands it.
pub const USED_TARGET: usize = 0;
pub const USED_POINT: usize = 1;
pub const USED_RANK: usize = 2;

/// An item or a skill was used.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Debug)]
pub struct Used {}

impl EventDef for Used {
    fn owners(&self) -> &'static [RunnerKind] {
        &[ITEM_RUNNER, SKILL_RUNNER]
    }

    fn fields(&self) -> &'static [(&'static str, ValueType)] {
        USED_FIELDS
    }
}

/// An item was picked up off the ground.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Debug)]
pub struct PickedUp {}

impl EventDef for PickedUp {
    fn owners(&self) -> &'static [RunnerKind] {
        &[ITEM_RUNNER]
    }
}

pub fn scripting(vocabulary: &mut Vocabulary) {
    query::scripting(vocabulary);
    vocabulary
        .runner(
            ITEM_RUNNER,
            RunnerSpec::named_row::<ItemDef>()
                .stateless()
                .starting_with_world(),
        )
        .binder(|loading, content, bind| bind.bind_table::<ItemDef>(loading, content))
        .event::<Used>("used")
        .event::<PickedUp>("picked_up");
    effects::scripting(vocabulary);
}
