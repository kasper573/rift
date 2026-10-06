pub mod card;
pub mod render;
pub mod widget;

use bevy_app::App;
use bevy_ecs::component::Component;
use bevy_ecs::entity::{Entity, MapEntities};
use bevy_ecs::message::Message;
use bevy_ecs::query::{QueryState, With};
use bevy_ecs::world::World;
use bevy_replicon::prelude::{Replicated, SendTargets, ToClients};
use bevy_time::Time;
use serde::{Deserialize, Serialize};

use crate::core::assets::{AssetRef, AssetService};
use crate::core::math::{Offset, Pos};
use crate::core::sfx::SfxId;
use crate::core::tiling::{TilePos, Tiles};
use crate::core::time::{Seconds, WallClock};
use crate::data::item::Id;
use crate::systems::area::{self, AreaTag};
use crate::systems::effect::{self, Effect, TimedEffect, TimedEffects};
use crate::systems::equipment;
use crate::systems::movement::{Position, position};
use crate::systems::notification::{self, Notification, NotificationKind};
use crate::systems::npc::Npc;
use crate::systems::player::{ClientId, Owner, commands_locked, conn_player, sender_player};
use crate::systems::reach::{self, Pursuit, ReachAct};
use crate::systems::rule::{Encounter, Outcome, Requirement, RuleContext, Terms};
use crate::systems::stat;
use crate::systems::text::LineText;
use crate::systems::visibility::{self, Presence, seen_by};

pub const INVENTORY_MAX: u32 = 25;
const RESERVATION_TTL: Seconds = Seconds(60.0);
const DROP_TTL: Seconds = Seconds(120.0);
const DROP_RADIUS: Tiles = Tiles(1.0);
const PICKUP_RANGE: Tiles = Tiles(1.0);

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;

    app.replicate::<Inventory>()
        .replicate::<DroppedItem>()
        .replicate::<Reservation>()
        .add_client_message::<UseItemRequest>(Channel::Ordered)
        .add_client_message::<DropItemRequest>(Channel::Ordered)
        .add_mapped_client_message::<PickupRequest>(Channel::Ordered)
        .add_mapped_server_message::<ItemConsumed>(Channel::Ordered)
        .add_mapped_server_message::<ItemsDropped>(Channel::Ordered);
    effect::source(app, carried);
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Inventory {
    pub slots: Vec<ItemStack>,
    pub max: u32,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemStack {
    pub item: Id,
    pub count: u32,
}

impl ItemStack {
    pub const fn new(item: Id, count: u32) -> ItemStack {
        ItemStack { item, count }
    }

    pub fn describe(self) -> String {
        format!("{} {}", self.count, self.item.get().display_name)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExchangeRefusal {
    Missing(ItemStack),
    NoRoom { slots: u32 },
}

impl ExchangeRefusal {
    pub fn describe(self) -> String {
        match self {
            ExchangeRefusal::Missing(stack) => {
                format!(
                    "Needs {} more {}",
                    stack.count,
                    stack.item.get().display_name
                )
            }
            ExchangeRefusal::NoRoom { slots: 1 } => "Needs 1 free slot".to_owned(),
            ExchangeRefusal::NoRoom { slots } => format!("Needs {slots} free slots"),
        }
    }
}

impl Inventory {
    pub fn empty() -> Inventory {
        Inventory {
            slots: Vec::new(),
            max: INVENTORY_MAX,
        }
    }

    pub fn count(&self, item: Id) -> u32 {
        self.slots
            .iter()
            .filter(|slot| slot.item == item)
            .fold(0, |total, slot| total.saturating_add(slot.count))
    }

    pub fn free_slots(&self) -> u32 {
        self.max.saturating_sub(self.slots.len() as u32)
    }

    pub fn exchange(
        &mut self,
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
        let mut unbounded = next.clone();
        unbounded.max = u32::MAX;
        for give in gives {
            unbounded.add(give.item, give.count);
        }
        let needed = unbounded.slots.len() as u32;
        if needed > next.max {
            return Err(ExchangeRefusal::NoRoom {
                slots: needed - next.max,
            });
        }
        *self = unbounded;
        self.max = next.max;
        Ok(())
    }

    fn add(&mut self, item: Id, mut count: u32) {
        let stack_max = item.get().stack_max();
        for slot in self.slots.iter_mut().filter(|slot| slot.item == item) {
            if count == 0 {
                break;
            }
            let take = stack_max.saturating_sub(slot.count).min(count);
            slot.count += take;
            count -= take;
        }
        while count > 0 && (self.slots.len() as u32) < self.max {
            let take = stack_max.min(count);
            self.slots.push(ItemStack::new(item, take));
            count -= take;
        }
    }

    fn remove(&mut self, stack: ItemStack) {
        let mut left = stack.count;
        for slot in self
            .slots
            .iter_mut()
            .rev()
            .filter(|slot| slot.item == stack.item)
        {
            let take = slot.count.min(left);
            slot.count -= take;
            left -= take;
        }
        self.slots.retain(|slot| slot.count > 0);
    }
}

fn merged(stacks: &[ItemStack]) -> Vec<ItemStack> {
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

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReservedBy {
    None,
    Account(ClientId),
}

impl ReservedBy {
    pub fn allows(self, account: Option<ClientId>) -> bool {
        match self {
            ReservedBy::None => true,
            ReservedBy::Account(client) => account == Some(client),
        }
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Reservation {
    pub by: ReservedBy,
    pub at: Seconds,
}

impl Reservation {
    pub fn expired(&self, now: Seconds) -> bool {
        now - self.at >= RESERVATION_TTL
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DroppedItem {
    pub item: Id,
    pub count: u32,
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct UseItemRequest {
    pub slot: u32,
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DropItemRequest {
    pub slot: u32,
}

#[derive(Message, Serialize, Deserialize, MapEntities, Clone, Debug, PartialEq)]
pub struct PickupRequest {
    #[entities]
    pub target: Entity,
}

#[derive(Message, Serialize, Deserialize, MapEntities, Clone, Debug, PartialEq)]
pub struct ItemConsumed {
    pub item: Id,
    #[entities]
    pub actor: Entity,
}

#[derive(Message, Serialize, Deserialize, MapEntities, Clone, Debug, PartialEq)]
pub struct ItemsDropped {
    #[entities]
    pub items: Vec<Entity>,
    pub from: Pos<Tiles>,
}

pub struct ItemDef {
    pub display_name: &'static str,
    pub flavor: &'static str,
    pub icon: AssetRef,
    pub sfx: ItemSfx,
    pub stackable: Option<Stackable>,
    pub effects: &'static [Effect],
    pub kind: ItemKind,
    pub flags: &'static [ItemFlag],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemFlag {
    Quest,
    Bound,
    Currency,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemCategory {
    Consumable,
    Equipment,
    Material,
    Currency,
}

impl ItemCategory {
    pub fn label(self) -> &'static str {
        match self {
            ItemCategory::Consumable => "Consumable",
            ItemCategory::Equipment => "Equipment",
            ItemCategory::Material => "Material",
            ItemCategory::Currency => "Currency",
        }
    }

    pub fn plural(self) -> &'static str {
        match self {
            ItemCategory::Consumable => "consumables",
            ItemCategory::Equipment => "equipment",
            ItemCategory::Material => "materials",
            ItemCategory::Currency => "currency",
        }
    }
}

impl ItemDef {
    pub fn stack_max(&self) -> u32 {
        self.stackable.map_or(1, |stackable| stackable.max)
    }

    pub fn has(&self, flag: ItemFlag) -> bool {
        self.flags.contains(&flag)
    }

    pub fn bound(&self) -> bool {
        self.has(ItemFlag::Bound) || self.has(ItemFlag::Quest)
    }

    pub fn category(&self) -> ItemCategory {
        if self.has(ItemFlag::Currency) {
            return ItemCategory::Currency;
        }
        match self.kind {
            ItemKind::Consumable { .. } => ItemCategory::Consumable,
            ItemKind::Equipment { .. } => ItemCategory::Equipment,
            ItemKind::Resource | ItemKind::Usable { .. } => ItemCategory::Material,
        }
    }

    fn use_from(&self, ctx: &mut UseCtx) {
        match &self.kind {
            ItemKind::Consumable {
                health_bonus,
                duration,
            } => {
                ctx.heal(*health_bonus);
                ctx.consume();
                ctx.apply_effects(*duration);
            }
            ItemKind::Equipment { slot, requirements } => {
                ctx.equip(*slot, requirements);
            }
            ItemKind::Usable { then } => ctx.settle(then),
            ItemKind::Resource => {}
        }
    }
}

pub enum ItemKind {
    Consumable {
        health_bonus: f32,
        duration: Seconds,
    },
    Equipment {
        slot: equipment::EquipmentSlot,
        requirements: &'static [&'static dyn Requirement],
    },
    Usable {
        then: &'static [&'static dyn Outcome],
    },
    Resource,
}

impl ItemKind {
    fn carried(&self) -> bool {
        matches!(self, ItemKind::Resource)
    }
}

pub struct ItemSfx {
    pub on_use: Option<SfxId>,
    pub pickup: SfxId,
    pub trade: SfxId,
    pub drop: SfxId,
}

#[derive(Clone, Copy)]
pub struct Stackable {
    pub max: u32,
}

pub struct UseCtx<'a> {
    world: &'a mut World,
    actor: Entity,
    slot: usize,
    item: Id,
}

impl UseCtx<'_> {
    pub fn heal(&mut self, amount: f32) {
        stat::heal(self.world, self.actor, amount);
    }
    pub fn consume(&mut self) {
        consume_slot(self.world, self.actor, self.slot);
        announce_consumed(self.world, self.actor, self.item);
    }
    pub fn apply_effects(&mut self, duration: Seconds) {
        instantiate_effects(self.world, self.actor, self.item, duration);
    }
    pub fn settle(&mut self, then: &'static [&'static dyn Outcome]) {
        let terms = Terms {
            requires: &[],
            costs: &[],
            outcomes: then,
        };
        if let Err(refusal) = terms.settle(self.world, self.actor, Encounter::default()) {
            notification::notify(
                self.world,
                self.actor,
                Notification::new(NotificationKind::error(), LineText::plain(refusal.0)),
            );
        }
    }
    pub fn equip(
        &mut self,
        into: equipment::EquipmentSlot,
        requirements: &'static [&'static dyn Requirement],
    ) {
        equipment::equip(self.world, self.actor, self.slot, into, requirements);
    }
}

pub fn use_item(world: &mut World) {
    for request in crate::systems::requests::<UseItemRequest>(world) {
        let Some(entity) = sender_player(world, request.client_id) else {
            continue;
        };
        if stat::is_dead(world, entity) {
            continue;
        }
        let slot = request.message.slot as usize;
        let Some(item) = world
            .get::<Inventory>(entity)
            .and_then(|inventory| inventory.slots.get(slot).map(|slot| slot.item))
        else {
            continue;
        };
        item.get().use_from(&mut UseCtx {
            world,
            actor: entity,
            slot,
            item,
        });
    }
}

pub fn drop_item(world: &mut World) {
    for request in crate::systems::requests::<DropItemRequest>(world) {
        let Some(player) = sender_player(world, request.client_id) else {
            continue;
        };
        if stat::is_dead(world, player) {
            continue;
        }
        let slot = request.message.slot as usize;
        let removed = match world.get_mut::<Inventory>(player) {
            Some(mut inventory)
                if inventory
                    .slots
                    .get(slot)
                    .is_some_and(|stack| !stack.item.get().bound()) =>
            {
                Some(inventory.slots.remove(slot))
            }
            _ => None,
        };
        let Some(slot) = removed else {
            continue;
        };
        scatter_drop(
            world,
            player,
            &[(slot.item, slot.count)],
            ReservedBy::None,
            None,
        );
    }
}

pub fn pickup_request(world: &mut World) {
    for request in crate::systems::requests::<PickupRequest>(world) {
        let Some(player) = sender_player(world, request.client_id) else {
            continue;
        };
        let target = request.message.target;
        if stat::is_dead(world, player)
            || commands_locked(world, player)
            || world.get::<DroppedItem>(target).is_none()
            || !visibility::present(world, target, player)
        {
            continue;
        }
        reach::intend(world, player, target, ReachAct::Pickup);
    }
}

pub fn pickable_at(world: &mut World, point: Pos<Tiles>) -> Option<Entity> {
    let me = crate::systems::player::session::my_id(world);
    let cell = point.cell();
    world
        .query::<(Entity, &Position, &DroppedItem, Option<&Reservation>)>()
        .iter(world)
        .find_map(|(entity, at, _, reservation)| {
            let reserved = reservation.map_or(ReservedBy::None, |reservation| reservation.by);
            (at.pos.cell() == cell && reserved.allows(me)).then_some(entity)
        })
}

pub fn pickups(world: &mut World, intents: &mut reach::Intents) {
    for (player, target) in reach::intending(world, intents, ReachAct::Pickup) {
        match reach::pursue(world, player, PICKUP_RANGE) {
            Pursuit::Approaching => {}
            Pursuit::Lost => reach::forget(world, player),
            Pursuit::Arrived => {
                collect(world, player, target);
                reach::forget(world, player);
            }
        }
    }
}

pub fn expire_drops(
    world: &mut World,
    drops: &mut QueryState<(Entity, &'static Reservation), With<DroppedItem>>,
) {
    let now = Seconds(world.resource::<Time>().elapsed_secs());
    let stale: Vec<Entity> = drops
        .iter(world)
        .filter(|(_, reservation)| now - reservation.at >= DROP_TTL)
        .map(|(entity, _)| entity)
        .collect();
    for entity in stale {
        world.entity_mut(entity).despawn();
    }
}

pub fn reserve(world: &mut World, npc: Entity, attacker: Entity, now: Seconds) {
    let Some(account) = world.get::<Owner>(attacker).map(|owner| owner.client) else {
        return;
    };
    if world.get::<Npc>(npc).is_none() {
        return;
    }
    let claim = match world.get::<Reservation>(npc) {
        None => true,
        Some(reservation) => {
            reservation.by == ReservedBy::Account(account) || reservation.expired(now)
        }
    };
    if claim {
        world.entity_mut(npc).insert(Reservation {
            by: ReservedBy::Account(account),
            at: now,
        });
    }
}

pub fn scatter_drop(
    world: &mut World,
    source: Entity,
    drops: &[(Id, u32)],
    reserved_by: ReservedBy,
    presence: Option<Presence>,
) {
    if drops.is_empty() {
        return;
    }
    let Some(from) = position(world, source) else {
        return;
    };
    let Some(area_id) = world.get::<AreaTag>(source).map(|tag| tag.area) else {
        return;
    };
    let area = world
        .resource::<AssetService>()
        .resolve(area_id.get().map, area::build_area);
    let now = Seconds(world.resource::<Time>().elapsed_secs());
    let count = drops.len();
    let mut items = Vec::with_capacity(count);
    for (index, &(item, stack)) in drops.iter().enumerate() {
        let pos = scatter_pos(from, index, count, area);
        let entity = world
            .spawn((
                Replicated,
                Position { pos },
                AreaTag { area: area_id },
                DroppedItem { item, count: stack },
                Reservation {
                    by: reserved_by,
                    at: now,
                },
            ))
            .id();
        if let Some(presence) = presence {
            world.entity_mut(entity).insert(presence);
        }
        items.push(entity);
    }
    for client in seen_by(world, items[0]) {
        world.write_message(ToClients {
            targets: SendTargets::Single(bevy_replicon::prelude::ClientId::Client(client)),
            message: ItemsDropped {
                items: items.clone(),
                from,
            },
        });
    }
}

fn carried(world: &World, entity: Entity) -> Vec<Effect> {
    world
        .get::<Inventory>(entity)
        .map(|inventory| {
            inventory
                .slots
                .iter()
                .map(|slot| slot.item.get())
                .filter(|def| def.kind.carried())
                .flat_map(|def| def.effects.iter().copied())
                .collect()
        })
        .unwrap_or_default()
}

fn announce_consumed(world: &mut World, actor: Entity, item: Id) {
    for client in seen_by(world, actor) {
        world.write_message(ToClients {
            targets: SendTargets::Single(bevy_replicon::prelude::ClientId::Client(client)),
            message: ItemConsumed { item, actor },
        });
    }
}

fn consume_slot(world: &mut World, entity: Entity, slot: usize) {
    if let Some(mut inventory) = world.get_mut::<Inventory>(entity)
        && let Some(stack) = inventory.slots.get_mut(slot)
    {
        stack.count -= 1;
        if stack.count == 0 {
            inventory.slots.remove(slot);
        }
    }
}

fn collect(world: &mut World, player: Entity, item: Entity) {
    let Some(drop) = world.get::<DroppedItem>(item).cloned() else {
        return;
    };
    let reserved = world
        .get::<Reservation>(item)
        .map_or(ReservedBy::None, |reservation| reservation.by);
    let account = world.get::<Owner>(player).map(|owner| owner.client);
    if !reserved.allows(account) {
        return;
    }
    let collected = world
        .get_mut::<Inventory>(player)
        .is_some_and(|mut inventory| {
            inventory
                .exchange(&[], &[ItemStack::new(drop.item, drop.count)])
                .is_ok()
        });
    if collected {
        world.entity_mut(item).despawn();
    }
}

fn instantiate_effects(world: &mut World, actor: Entity, item: Id, duration: Seconds) {
    let effects = item.get().effects;
    if effects.is_empty() {
        return;
    }
    let until = world.resource::<WallClock>().now.after(duration);
    let entries = effects
        .iter()
        .map(|&effect| TimedEffect { effect, until })
        .collect::<Vec<_>>();
    match world.get_mut::<TimedEffects>(actor) {
        Some(mut timed) => timed.0.extend(entries),
        None => {
            world.entity_mut(actor).insert(TimedEffects(entries));
        }
    }
}

fn scatter_pos(from: Pos<Tiles>, index: usize, count: usize, area: &area::Area) -> Pos<Tiles> {
    let rest = from.snap();
    if count == 1 {
        return area.grid.nearest_walkable(from).unwrap_or(rest);
    }
    let angle = std::f32::consts::TAU * index as f32 / count as f32;
    let spread = from + Offset::new(angle.cos() * DROP_RADIUS.0, angle.sin() * DROP_RADIUS.0);
    area.grid.nearest_walkable(spread).unwrap_or(rest)
}

pub struct Holding(pub ItemStack);

impl Requirement for Holding {
    fn met(&self, world: &World, player: Entity) -> bool {
        world
            .get::<Inventory>(player)
            .is_some_and(|inventory| inventory.count(self.0.item) >= self.0.count)
    }

    fn describe(&self) -> String {
        self.0.describe()
    }
}

pub struct FreeSlots(pub u32);

impl Requirement for FreeSlots {
    fn met(&self, world: &World, player: Entity) -> bool {
        world
            .get::<Inventory>(player)
            .is_some_and(|inventory| inventory.free_slots() >= self.0)
    }

    fn describe(&self) -> String {
        match self.0 {
            1 => "1 free slot".to_owned(),
            slots => format!("{slots} free slots"),
        }
    }
}

pub struct GiveItems(pub &'static [ItemStack]);

impl Outcome for GiveItems {
    fn apply(&self, _ctx: &mut RuleContext) {}

    fn gives(&self) -> &[ItemStack] {
        self.0
    }
}

impl bevy_terminal::CommandArg for Id {
    fn parse(name: &str, raw: Option<&str>) -> Result<Id, String> {
        crate::core::table::parse_id(name, raw, "item")
    }
}

/// Give items to a player.
#[bevy_terminal::command(name = "give", access = crate::systems::account::role::is_admin)]
fn give(
    world: &mut World,
    ctx: &bevy_terminal::CommandCtx,
    item: Id,
    count: Option<u32>,
    user: Option<String>,
) -> Result<String, String> {
    let target = match &user {
        Some(user) => crate::systems::player::by_user(world, user)
            .ok_or_else(|| format!("no player with user id `{user}` in your area"))?,
        None => conn_player(world, ctx.conn).ok_or_else(|| "you have no player".to_owned())?,
    };
    let stack = ItemStack::new(item, count.unwrap_or(1));
    let mut inventory = world
        .get_mut::<Inventory>(target)
        .ok_or_else(|| "the player has no bag".to_owned())?;
    inventory
        .exchange(&[], &[stack])
        .map_err(ExchangeRefusal::describe)?;
    Ok(format!("gave {}", stack.describe()))
}
