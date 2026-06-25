//! Items: the [`ItemDef`] catalog, the replicated [`Inventory`] of stacked [`Slot`]s a character
//! carries, the [`DroppedItem`] entities loot spawns onto the map, the [`Reservation`] that makes a
//! kill's loot fair, and the use/drop/pickup request/announce messages with their server systems.

use std::sync::OnceLock;

use bevy_app::App;
use bevy_ecs::component::Component;
use bevy_ecs::entity::{Entity, MapEntities};
use bevy_ecs::message::Message;
use serde::{Deserialize, Deserializer, Serialize};

use crate::core::assets;
use crate::core::math::{Offset, Pos};
use crate::core::table::{self, Content, Id};
use crate::core::tiling::{TilePos, Tiles};
use crate::core::time::Seconds;
use crate::systems::sfx::SfxId;

use crate::systems::area::{self, AreaTag};
use crate::systems::combat::{Vitals, is_dead};
use crate::systems::movement::{MoveTarget, Position, goto, position};
use crate::systems::npc::Npc;
use crate::systems::player::{ClientId, Owner, sender_player};
use crate::systems::visibility::seen_by;
use bevy_ecs::message::Messages;
use bevy_ecs::query::With;
use bevy_ecs::world::World;
use bevy_replicon::prelude::{FromClient, Replicated, SendTargets, ToClients};
use bevy_time::Time;

const FILE: &str = "item_table.json";

pub const INVENTORY_MAX: u32 = 25;
const RESERVATION_TTL: Seconds = Seconds(60.0);
const DROP_TTL: Seconds = Seconds(120.0);
const DROP_RADIUS: Tiles = Tiles(1.0);
const PICKUP_RANGE: i32 = 1;

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
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Inventory {
    pub slots: Vec<Slot>,
    pub max: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Slot {
    pub item: Id<ItemDef>,
    pub count: u32,
}

impl Inventory {
    pub fn empty() -> Inventory {
        Inventory {
            slots: Vec::new(),
            max: INVENTORY_MAX,
        }
    }

    /// How many more of `item` the inventory can take across its existing stacks and free slots.
    pub fn capacity_for(&self, item: Id<ItemDef>) -> u32 {
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

    /// Adds `count` of `item`, filling matching stacks before opening new slots. The caller is
    /// responsible for first checking [`Inventory::capacity_for`]; any overflow is dropped.
    pub fn add(&mut self, item: Id<ItemDef>, mut count: u32) {
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
            self.slots.push(Slot { item, count: take });
            count -= take;
        }
    }
}

/// Who a [`Reservation`] belongs to. [`ClientId`] is the account, so a reserved drop is reachable
/// from any of that account's connections.
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

/// A timed claim, replicated so a client can tell which drops it may take. On an NPC it's the loot
/// reservation: the first attacker claims it, the reserved player's attacks refresh `at`, and it
/// lapses (for reward purposes) after [`RESERVATION_TTL`]. On a [`DroppedItem`] `at` is when it was
/// dropped, so [`expire_drops`] can clear it once it has lain unclaimed for [`DROP_TTL`].
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
    pub item: Id<ItemDef>,
    pub count: u32,
}

/// A player walking to a clicked [`DroppedItem`] to pick it up. Server-only.
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
    pub item: Id<ItemDef>,
    #[entities]
    pub actor: Entity,
}

/// Announces a fresh scatter of drops to the observers present, so their clients fountain the items
/// out from `from`. The drops themselves arrive by replication, so a late observer still sees them.
#[derive(Message, Serialize, Deserialize, MapEntities, Clone, Debug, PartialEq)]
pub struct ItemsDropped {
    #[entities]
    pub items: Vec<Entity>,
    pub from: Pos<Tiles>,
}

#[derive(Deserialize)]
pub struct ItemDef {
    pub id: String,
    pub display_name: String,
    pub icon: Icon,
    #[serde(default)]
    pub sfx: ItemSfx,
    #[serde(default)]
    pub stackable: Option<Stackable>,
    #[serde(flatten)]
    pub kind: ItemKind,
}

impl ItemDef {
    pub fn stack_max(&self) -> u32 {
        self.stackable.map_or(1, |stackable| stackable.max)
    }
}

impl Content for ItemDef {
    fn table() -> &'static [ItemDef] {
        items()
    }
    fn id(&self) -> &str {
        &self.id
    }
}

#[derive(Deserialize, Default)]
pub struct ItemSfx {
    #[serde(default, rename = "use")]
    pub on_use: Option<SfxId>,
    #[serde(default)]
    pub drop: Option<SfxId>,
}

#[derive(Deserialize, Clone, Copy)]
pub struct Stackable {
    pub max: u32,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ItemKind {
    Consumable { health_bonus: f32 },
    Resource,
    Equipment,
}

pub struct Icon(pub String);

impl<'de> Deserialize<'de> for Icon {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        assets::find(assets::ICONS, &format!("{name}.png"))
            .map(Icon)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown icon '{name}'")))
    }
}

pub fn items() -> &'static [ItemDef] {
    static ITEMS: OnceLock<Vec<ItemDef>> = OnceLock::new();
    ITEMS.get_or_init(|| {
        let items: Vec<ItemDef> = table::load(FILE);
        table::unique_ids(items.iter().map(|item| item.id.as_str()), FILE);
        items
    })
}

pub fn use_item(world: &mut World) {
    let requests: Vec<FromClient<UseItemRequest>> = world
        .resource_mut::<Messages<FromClient<UseItemRequest>>>()
        .drain()
        .collect();
    for request in requests {
        let Some(entity) = sender_player(world, request.client_id) else {
            continue;
        };
        if is_dead(world, entity) {
            continue;
        }
        let slot = request.message.slot as usize;
        let Some(item) = world
            .get::<Inventory>(entity)
            .and_then(|inventory| inventory.slots.get(slot).map(|slot| slot.item))
        else {
            continue;
        };
        match item.get().kind {
            ItemKind::Consumable { health_bonus } => {
                if let Some(mut vitals) = world.get_mut::<Vitals>(entity) {
                    vitals.heal(health_bonus);
                }
                consume_slot(world, entity, slot);
            }
            ItemKind::Resource | ItemKind::Equipment => continue,
        }
        // Announced per beholder: a mapped message only decodes for clients that see the actor.
        for client in seen_by(world, entity) {
            world.write_message(ToClients {
                targets: SendTargets::Single(bevy_replicon::prelude::ClientId::Client(client)),
                message: ItemConsumed {
                    item,
                    actor: entity,
                },
            });
        }
    }
}

pub fn drop_item(world: &mut World) {
    let requests: Vec<FromClient<DropItemRequest>> = world
        .resource_mut::<Messages<FromClient<DropItemRequest>>>()
        .drain()
        .collect();
    for request in requests {
        let Some(player) = sender_player(world, request.client_id) else {
            continue;
        };
        if is_dead(world, player) {
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
    let requests: Vec<FromClient<PickupRequest>> = world
        .resource_mut::<Messages<FromClient<PickupRequest>>>()
        .drain()
        .collect();
    for request in requests {
        let Some(player) = sender_player(world, request.client_id) else {
            continue;
        };
        let target = request.message.target;
        if is_dead(world, player) || world.get::<DroppedItem>(target).is_none() {
            continue;
        }
        let Some(at) = position(world, target) else {
            continue;
        };
        goto(world, player, at);
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
        if is_dead(world, player) || world.get::<DroppedItem>(target).is_none() {
            world.entity_mut(player).remove::<PickupIntent>();
            continue;
        }
        let (Some(at), Some(item_at)) = (position(world, player), position(world, target)) else {
            world.entity_mut(player).remove::<PickupIntent>();
            continue;
        };
        if chebyshev(at, item_at) <= PICKUP_RANGE {
            collect(world, player, target);
            world.entity_mut(player).remove::<PickupIntent>();
        } else if world
            .get::<MoveTarget>(player)
            .is_none_or(|goal| goal.pos.cell() != item_at.cell())
        {
            world.entity_mut(player).remove::<PickupIntent>();
        }
    }
}

/// Despawns dropped items that have lain on the ground past [`DROP_TTL`], measured from when the
/// drop stamped its [`Reservation`].
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

/// Claims or refreshes `npc`'s reservation for `attacker`'s account: a fresh, lapsed, or
/// already-held reservation goes to the attacker; a live reservation held by someone else stands.
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

/// Scatters `drops` onto walkable tiles in a circle around `source`, then tells every observer of
/// `source` to fountain them out. Each entry becomes one [`DroppedItem`] entity carrying its count.
pub fn scatter_drop(
    world: &mut World,
    source: Entity,
    drops: &[(Id<ItemDef>, u32)],
    reserved_by: ReservedBy,
) {
    if drops.is_empty() {
        return;
    }
    let Some(from) = position(world, source) else {
        return;
    };
    let area_id = world
        .get::<AreaTag>(source)
        .map_or(Id::new(0), |tag| tag.area);
    let area = &area::areas()[area_id.index()];
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

fn scatter_pos(from: Pos<Tiles>, index: usize, count: usize, area: &area::Area) -> Pos<Tiles> {
    let rest = from.snap();
    if count == 1 {
        return area.grid.nearest_walkable(from).unwrap_or(rest);
    }
    let angle = std::f32::consts::TAU * index as f32 / count as f32;
    let spread = from + Offset::new(angle.cos() * DROP_RADIUS.0, angle.sin() * DROP_RADIUS.0);
    area.grid.nearest_walkable(spread).unwrap_or(rest)
}

fn chebyshev(a: Pos<Tiles>, b: Pos<Tiles>) -> i32 {
    let (a, b) = (a.cell(), b.cell());
    (a.x - b.x).abs().max((a.y - b.y).abs())
}
