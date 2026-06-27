use bevy_app::App;
use bevy_ecs::component::Component;
use bevy_ecs::entity::{Entity, MapEntities};
use bevy_ecs::message::Message;
use serde::{Deserialize, Serialize};

use crate::core::assets;
use crate::core::math::{Offset, Pos};
use crate::core::tiling::{TilePos, Tiles};
use crate::core::time::Seconds;
use crate::data::item::Id;

use crate::systems::area::{self, AreaTag};
use crate::systems::effect::{self, Effect, TimedEffect, TimedEffects};
use crate::systems::equipment::{self, Requirement};
use crate::systems::movement::{MoveTarget, Position, approach, forget, position};
use crate::systems::npc::Npc;
use crate::systems::player::{ClientId, Owner, sender_player};
use crate::systems::sfx::SfxId;
use crate::systems::stat;
use crate::systems::visibility::seen_by;
use bevy_ecs::query::With;
use bevy_ecs::world::World;
use bevy_replicon::prelude::{Replicated, SendTargets, ToClients};
use bevy_time::Time;

pub const INVENTORY_MAX: u32 = 25;
const RESERVATION_TTL: Seconds = Seconds(60.0);
const DROP_TTL: Seconds = Seconds(120.0);
const DROP_RADIUS: Tiles = Tiles(1.0);
const PICKUP_RANGE: Tiles = Tiles(std::f32::consts::SQRT_2);

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

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Inventory {
    pub slots: Vec<InventorySlot>,
    pub max: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct InventorySlot {
    pub item: Id,
    pub count: u32,
}

impl Inventory {
    pub fn empty() -> Inventory {
        Inventory {
            slots: Vec::new(),
            max: INVENTORY_MAX,
        }
    }

    pub fn capacity_for(&self, item: Id) -> u32 {
        let stack_max = item.get().stack_max();
        let in_existing: u32 = self
            .slots
            .iter()
            .filter(|slot| slot.item == item)
            .map(|slot| stack_max.saturating_sub(slot.count))
            .sum();
        let free_slots = (self.max as usize).saturating_sub(self.slots.len()) as u32;
        in_existing + free_slots * stack_max
    }

    pub fn add(&mut self, item: Id, mut count: u32) {
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
            self.slots.push(InventorySlot { item, count: take });
            count -= take;
        }
    }
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

#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct PickupIntent {
    pub target: Entity,
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
    pub icon: Icon,
    pub sfx: ItemSfx,
    pub stackable: Option<Stackable>,
    pub effects: &'static [Effect],
    pub kind: ItemKind,
}

impl ItemDef {
    pub fn stack_max(&self) -> u32 {
        self.stackable.map_or(1, |stackable| stackable.max)
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
        slot: equipment::Slot,
        requirements: &'static [Requirement],
    },
    Resource,
}

impl ItemKind {
    fn carried(&self) -> bool {
        matches!(self, ItemKind::Resource)
    }
}

#[derive(Default)]
pub struct ItemSfx {
    pub on_use: Option<SfxId>,
    pub drop: Option<SfxId>,
}

#[derive(Clone, Copy)]
pub struct Stackable {
    pub max: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Icon(pub &'static str);

impl Icon {
    pub fn path(self) -> String {
        assets::find(assets::ICONS, &format!("{}.png", self.0))
            .unwrap_or_else(|| panic!("unknown icon '{}'", self.0))
    }
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
    pub fn equip(&mut self, into: equipment::Slot, requirements: &[Requirement]) {
        equipment::equip(self.world, self.actor, self.slot, into, requirements);
    }
}

fn announce_consumed(world: &mut World, actor: Entity, item: Id) {
    for client in seen_by(world, actor) {
        world.write_message(ToClients {
            targets: SendTargets::Single(bevy_replicon::prelude::ClientId::Client(client)),
            message: ItemConsumed { item, actor },
        });
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
            Some(mut inventory) if slot < inventory.slots.len() => {
                Some(inventory.slots.remove(slot))
            }
            _ => None,
        };
        let Some(slot) = removed else {
            continue;
        };
        scatter_drop(world, player, &[(slot.item, slot.count)], ReservedBy::None);
    }
}

pub fn pickup_request(world: &mut World) {
    for request in crate::systems::requests::<PickupRequest>(world) {
        let Some(player) = sender_player(world, request.client_id) else {
            continue;
        };
        let target = request.message.target;
        if stat::is_dead(world, player) || world.get::<DroppedItem>(target).is_none() {
            continue;
        }
        let Some(at) = position(world, target) else {
            continue;
        };
        forget(world, player);
        approach(world, player, at, PICKUP_RANGE);
        world.entity_mut(player).insert(PickupIntent { target });
    }
}

pub fn pickups(world: &mut World) {
    let players: Vec<Entity> = world
        .query_filtered::<Entity, With<PickupIntent>>()
        .iter(world)
        .collect();
    for player in players {
        let Some(target) = world
            .get::<PickupIntent>(player)
            .map(|intent| intent.target)
        else {
            continue;
        };
        if stat::is_dead(world, player) || world.get::<DroppedItem>(target).is_none() {
            world.entity_mut(player).remove::<PickupIntent>();
            continue;
        }
        let (Some(at), Some(item_at)) = (position(world, player), position(world, target)) else {
            world.entity_mut(player).remove::<PickupIntent>();
            continue;
        };
        if at.distance(item_at) <= PICKUP_RANGE {
            collect(world, player, target);
            world.entity_mut(player).remove::<PickupIntent>();
        } else if world
            .get::<MoveTarget>(player)
            .is_none_or(|goal| goal.pos.distance(item_at) > PICKUP_RANGE)
        {
            world.entity_mut(player).remove::<PickupIntent>();
        }
    }
}

pub fn expire_drops(world: &mut World) {
    let now = Seconds(world.resource::<Time>().elapsed_secs());
    let stale: Vec<Entity> = world
        .query_filtered::<(Entity, &Reservation), With<DroppedItem>>()
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
) {
    if drops.is_empty() {
        return;
    }
    let Some(from) = position(world, source) else {
        return;
    };
    let Some(area) = area::of(world, source) else {
        return;
    };
    let area_id = area.id;
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
        items.push(entity);
    }
    for client in seen_by(world, source) {
        world.write_message(ToClients {
            targets: SendTargets::Single(bevy_replicon::prelude::ClientId::Client(client)),
            message: ItemsDropped {
                items: items.clone(),
                from,
            },
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
    let fits = world
        .get::<Inventory>(player)
        .is_some_and(|inventory| inventory.capacity_for(drop.item) >= drop.count);
    if !reserved.allows(account) || !fits {
        return;
    }
    if let Some(mut inventory) = world.get_mut::<Inventory>(player) {
        inventory.add(drop.item, drop.count);
    }
    world.entity_mut(item).despawn();
}

fn instantiate_effects(world: &mut World, actor: Entity, item: Id, duration: Seconds) {
    let effects = item.get().effects;
    if effects.is_empty() {
        return;
    }
    let until = Seconds(world.resource::<Time>().elapsed_secs()) + duration;
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
