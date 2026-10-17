pub mod session;
pub mod snapshot;
pub mod widget;

use std::collections::{HashMap, HashSet};

use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_replicon::prelude::{ClientId as Sender, Replicated};
use bevy_time::Time;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::{Content, ContentId, ContentRow, ModuleRule, ModuleSet};
use crate::core::math::Rgba;
use crate::core::math::{Direction, Pos, Rng};
use crate::core::tiling::{CellPos, TilePos, Tiles};
use crate::core::time::{PlaybackRate, Seconds, UnixMillis, WallClock};
use crate::systems::GameSettings;
use crate::systems::account::Identity;
use crate::systems::actor::model::ModelId;
use crate::systems::actor::{Action, Actor, ActorModel, Hitbox, Name, Tint};
use crate::systems::area::transition::{self, Discovered};
use crate::systems::area::{self, AreaId, AreaTag};
use crate::systems::belongings;
use crate::systems::class::{Class, ClassId};
use crate::systems::combat::{self, HealthRegen};
use crate::systems::dialogue::Heard;
use crate::systems::equipment::Equipment;
use crate::systems::history::History;
use crate::systems::item::{Inventory, ItemId};
use crate::systems::memory::Memory;
use crate::systems::movement::{HeardMove, Position};
use crate::systems::quest::QuestLog;
use crate::systems::script::{EventKind, Place};
use crate::systems::shop::{self, ShopLedger};
use crate::systems::skill::{BasicAttack, Cooldowns, Hotbar, Learned};
use crate::systems::stat::{self, StatKind, StatMap, Stats};
use crate::systems::visibility::{self, OwnedBy};
use crate::systems::{Character, WorldArea, drain_requests};
use snapshot::{CharacterSnapshot, JoinAs};

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;

    app.replicate_once::<Owner>()
        .replicate::<Xp>()
        .replicate::<CommandLock>()
        .replicate::<ArrivalHold>()
        .add_client_message::<JoinRequest>(Channel::Ordered)
        .add_client_message::<RespawnRequest>(Channel::Ordered)
        .add_client_message::<AreaDrawn>(Channel::Ordered)
        .add_server_message::<Welcome>(Channel::Ordered);
}

#[derive(
    Component,
    Serialize,
    Deserialize,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Debug,
    Default,
)]
#[component(immutable)]
pub struct ClientId(pub u32);

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Owner {
    pub client: ClientId,
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Xp {
    pub amount: u32,
}

impl Xp {
    pub fn gain(&mut self, amount: u32) {
        self.amount += amount;
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandLock;

#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct ArrivalHold {
    pub until: Seconds,
}

/// How long an arrival holds its character: the client reveals a view that is still not drawable
/// after this long, and reports the arrival, while the server releases the hold on its own if no
/// report comes in.
pub const ARRIVAL_PATIENCE: Seconds = Seconds(5.0);

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AreaDrawn;

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct JoinRequest;

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RespawnRequest;

#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Welcome {
    pub id: ClientId,
    pub now: UnixMillis,
}

/// High enough that nothing in the world can grind it down.
const IMMORTAL_HEALTH: f32 = 9999.0;

pub type PlayerDefId = ContentId<PlayerDef>;

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct PlayerDef {
    pub model: ModelId,
    pub class: ClassId,
    pub stats: StatMap,
    #[schemars(extend("x-usage" = "worn at creation"))]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub wearing: Vec<ItemId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modules: Vec<PlayerModule>,
}

impl ContentRow for PlayerDef {
    const TABLE: &'static str = "player";
}

impl ModuleSet for PlayerDef {
    type Module = PlayerModule;

    fn modules(&self) -> &[PlayerModule] {
        &self.modules
    }

    fn rule(_: &PlayerModule) -> ModuleRule {
        ModuleRule::AtMostOne
    }
}

#[derive(JsonSchema, Clone, Copy, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PlayerModule {
    Tint(Tint),
    Regen(HealthRegen),
}

impl PlayerDef {
    pub fn tint(&self) -> Rgba {
        self.modules
            .iter()
            .find_map(|module| match module {
                PlayerModule::Tint(tint) => Some(tint.rgba),
                _ => None,
            })
            .unwrap_or(Rgba::WHITE)
    }

    pub fn regen(&self) -> Option<HealthRegen> {
        self.modules.iter().find_map(|module| match module {
            PlayerModule::Regen(regen) => Some(*regen),
            _ => None,
        })
    }
}

pub fn def(content: &Content) -> &PlayerDef {
    content.built::<GameSettings>().player.get(content)
}

#[derive(Resource, Default)]
pub struct Players(pub HashMap<ClientId, Entity>);

pub(crate) fn sender_player(world: &World, sender: Sender) -> Option<Entity> {
    conn_player(world, sender.entity()?)
}

pub fn commands_locked(world: &World, player: Entity) -> bool {
    world.get::<CommandLock>(player).is_some() || held(world, player)
}

pub fn may_command(world: &World, player: Entity) -> bool {
    !stat::is_dead(world, player) && !commands_locked(world, player)
}

pub fn held(world: &World, entity: Entity) -> bool {
    world.get::<ArrivalHold>(entity).is_some()
}

pub fn arrival_holds(world: &mut World) {
    for request in drain_requests::<AreaDrawn>(world) {
        if let Some(player) = sender_player(world, request.client_id)
            && let Ok(mut player) = world.get_entity_mut(player)
        {
            player.remove::<ArrivalHold>();
        }
    }
    let now = Seconds(world.resource::<Time>().elapsed_secs());
    let expired: Vec<Entity> = world
        .query::<(Entity, &ArrivalHold)>()
        .iter(world)
        .filter(|(_, hold)| hold.until <= now)
        .map(|(entity, _)| entity)
        .collect();
    for entity in expired {
        world.entity_mut(entity).remove::<ArrivalHold>();
    }
}

pub(crate) fn conn_player(world: &World, conn: Entity) -> Option<Entity> {
    let client = world.get::<ClientId>(conn)?;
    world.resource::<Players>().0.get(client).copied()
}

pub fn greet(add: On<Add<ClientId>>, clients: Query<&ClientId>, mut commands: Commands) {
    let Ok(&id) = clients.get(add.entity) else {
        return;
    };
    let conn = add.entity;
    commands.queue(move |world: &mut World| {
        let now = world.resource::<WallClock>().now;
        visibility::send_to(world, conn, Welcome { id, now });
    });
}

pub fn client_left(
    remove: On<Remove<ClientId>>,
    clients: Query<(Entity, &ClientId)>,
    mut players: ResMut<Players>,
    mut commands: Commands,
) {
    if let Some(id) = last_connection(remove.entity, &clients)
        && let Some(entity) = players.0.remove(&id)
    {
        commands.entity(entity).despawn();
    }
}

/// The client a closing connection belongs to, unless the client is still connected another way.
pub(crate) fn last_connection(
    closing: Entity,
    clients: &Query<(Entity, &ClientId)>,
) -> Option<ClientId> {
    let (_, &id) = clients.get(closing).ok()?;
    let elsewhere = clients
        .iter()
        .any(|(conn, other)| conn != closing && *other == id);
    (!elsewhere).then_some(id)
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SpawnPolicy {
    Map,
    Dist,
}

#[derive(Resource, Clone, Copy)]
pub struct SpawnRules {
    pub policy: SpawnPolicy,
    pub immortal: bool,
}

pub fn join(world: &mut World) {
    let zone = world.resource::<WorldArea>().0;
    let content = world.resource::<Content>().clone();
    let area = area::load(&content, zone);
    let SpawnRules { policy, immortal } = *world.resource::<SpawnRules>();
    for request in drain_requests::<JoinRequest>(world) {
        let Some(client_entity) = request.client_id.entity() else {
            continue;
        };
        let Some(&client) = world.get::<ClientId>(client_entity) else {
            continue;
        };
        if world.resource::<Players>().0.contains_key(&client) {
            continue;
        }
        let clock = *world.resource::<WallClock>();
        let (at, state) = match world.get::<JoinAs>(client_entity) {
            Some(JoinAs { snapshot, start }) if start.area == zone => {
                let (snapshot, start) = (snapshot.clone(), *start);
                (
                    beside_anyone_standing_there(world, area, start.at),
                    snapshot::materialize(&content, &snapshot, start, clock, immortal),
                )
            }
            _ => {
                let name = world
                    .get::<Identity>(client_entity)
                    .map_or_else(|| format!("player {}", client.0), |id| id.name.clone());
                let at = spawn_position(world, policy, zone, area);
                let fresh = CharacterSnapshot::fresh(&content, &name, def(&content));
                let start = Place { area: zone, at };
                let state = snapshot::materialize(&content, &fresh, start, clock, immortal);
                (at, state)
            }
        };
        let entity = place(world, client, zone, at, state);
        spawned(world, entity);
    }
}

pub fn spawned(world: &mut World, player: Entity) {
    transition::raise_area(world, EventKind::Entered, player);
}

/// A character asked to start on a tile someone already stands on is put down on the nearest
/// free one instead, so neither hides the other.
fn beside_anyone_standing_there(
    world: &mut World,
    area: &area::Area,
    at: Pos<Tiles>,
) -> Pos<Tiles> {
    let taken: HashSet<CellPos> = world
        .query::<&Position>()
        .iter(world)
        .map(|position| position.pos.cell())
        .collect();
    area.grid
        .nearest_walkable_where(at, |cell| !taken.contains(&cell))
        .unwrap_or(at)
}

fn spawn_position(
    world: &mut World,
    policy: SpawnPolicy,
    zone: AreaId,
    area: &area::Area,
) -> Pos<Tiles> {
    let spawn = world.resource::<GameSettings>().initial_spawn;
    let fallback = || area.fallback_spawn();
    match policy {
        SpawnPolicy::Map if spawn.area == zone => spawn.at,
        SpawnPolicy::Map => fallback(),
        SpawnPolicy::Dist => area
            .grid
            .random_node(&mut world.resource_mut::<Rng>())
            .unwrap_or_else(fallback),
    }
}

#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct SpawnPoint(pub Pos<Tiles>);

#[derive(Bundle)]
pub struct CharacterState {
    pub name: Name,
    pub spawn: SpawnPoint,
    pub discovered: Discovered,
    pub stats: Stats,
    pub inventory: Inventory,
    pub xp: Xp,
    pub equipment: Equipment,
    pub class: Class,
    pub learned: Learned,
    pub cooldowns: Cooldowns,
    pub hotbar: Hotbar,
    pub basic_attack: BasicAttack,
    pub memory: Memory,
    pub heard: Heard,
    pub ledger: ShopLedger,
    pub quests: QuestLog,
    pub history: History,
}

impl CharacterState {
    pub fn of(world: &World, entity: Entity, spawn: Pos<Tiles>) -> Option<CharacterState> {
        Some(CharacterState {
            name: world.get::<Name>(entity)?.clone(),
            spawn: SpawnPoint(spawn),
            discovered: world.get::<Discovered>(entity)?.clone(),
            stats: stat::snapshot(world, entity),
            inventory: world.get::<Inventory>(entity)?.clone(),
            xp: world.get::<Xp>(entity)?.clone(),
            equipment: world.get::<Equipment>(entity)?.clone(),
            class: *world.get::<Class>(entity)?,
            learned: world.get::<Learned>(entity).cloned().unwrap_or_default(),
            cooldowns: world.get::<Cooldowns>(entity).cloned().unwrap_or_default(),
            hotbar: world.get::<Hotbar>(entity).cloned().unwrap_or_default(),
            basic_attack: world
                .get::<BasicAttack>(entity)
                .copied()
                .unwrap_or_default(),
            memory: world.get::<Memory>(entity).cloned().unwrap_or_default(),
            heard: world.get::<Heard>(entity).cloned().unwrap_or_default(),
            ledger: shop::ledger(world, entity),
            quests: world.get::<QuestLog>(entity).cloned().unwrap_or_default(),
            history: world.get::<History>(entity).cloned().unwrap_or_default(),
        })
    }
}

pub fn player_stats(stats: &stat::StatMap, immortal: bool) -> Stats {
    Stats(
        stats
            .listed()
            .map(|stat| match stat.kind {
                StatKind::Health | StatKind::MaxHealth if immortal => stat.kind.of(IMMORTAL_HEALTH),
                _ => stat,
            })
            .collect(),
    )
}

pub(crate) fn place(
    world: &mut World,
    client: ClientId,
    zone: AreaId,
    at: Pos<Tiles>,
    state: CharacterState,
) -> Entity {
    let content = world.resource::<Content>().clone();
    let def = def(&content);
    let model = def.model;
    let now = Seconds(world.resource::<Time>().elapsed_secs());
    let entity = world
        .spawn((
            Character {
                replicated: Replicated,
                position: Position::at(at),
                actor: Actor {
                    color: def.tint(),
                    dir: Direction::S,
                    action: Action::Idle,
                    model,
                    attack_rate: PlaybackRate::default(),
                },
                hitbox: Hitbox {
                    size: ActorModel::get(&content, model).hitbox(),
                },
                area: AreaTag { area: zone },
            },
            OwnedBy(client),
            Owner { client },
            HeardMove::default(),
            ArrivalHold {
                until: now + ARRIVAL_PATIENCE,
            },
            state,
        ))
        .id();
    let belongings = belongings::of(world, entity);
    world.entity_mut(entity).insert(belongings);
    world.resource_mut::<Players>().0.insert(client, entity);
    entity
}

pub fn respawn(world: &mut World) {
    let content = world.resource::<Content>().clone();
    for request in drain_requests::<RespawnRequest>(world) {
        let Some(entity) = sender_player(world, request.client_id) else {
            continue;
        };
        if !stat::is_dead(world, entity) {
            continue;
        }
        let Some(spawn) = world.get::<SpawnPoint>(entity).map(|spawn| spawn.0) else {
            continue;
        };
        if let Some(mut memory) = world.get_mut::<Memory>(entity) {
            memory.leave_area(&content);
        }
        combat::revive(world, entity, spawn);
    }
}
