pub mod session;
pub mod snapshot;
pub mod widget;

use std::collections::HashMap;

use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_replicon::prelude::ClientId as Sender;
use bevy_time::Time;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use snapshot::{CharacterSnapshot, JoinAs};

use crate::core::content::{Content, ContentFile, ContentId, ModuleRule, ModuleSet, Nearby};
use crate::core::geometry::Radians;
use crate::core::math::{Pos, Rgba, Rng};
use crate::core::time::{DayOrigin, PlaybackRate, Seconds, UnixMillis, WallClock};
use crate::core::units::{Meters, MetersPerSec};
use crate::systems::account::Identity;
use crate::systems::actor::model::ModelId;
use crate::systems::actor::{Action, Actor, ActorModel, Hitbox, Name, Pose, Tint};
use crate::systems::area::transition::{self, Discovered};
use crate::systems::area::{self, AreaId, AreaTag};
use crate::systems::class::{Class, ClassId};
use crate::systems::combat::{self, HealthRegen};
use crate::systems::dialogue::Heard;
use crate::systems::equipment::Equipment;
use crate::systems::history::History;
use crate::systems::item::{Inventory, ItemId};
use crate::systems::memory::Memory;
use crate::systems::movement::{self, HeardMove, Position};
use crate::systems::quest::QuestLog;
use crate::systems::script::{EventKind, Place};
use crate::systems::shop::{self, ShopLedger};
use crate::systems::skill::{BasicAttack, Hotbar, Learned};
use crate::systems::stat::{self, Immortal, Stats};
use crate::systems::visibility::{self, OwnedBy};
use crate::systems::{
    Character, GameSettings, WorldArea, belongings, drain_requests, notification,
};

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;

    app.replicate_once::<PlayedBy>()
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
pub struct PlayedBy {
    pub client: ClientId,
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Xp {
    pub amount: u32,
}

impl Xp {
    pub fn gain(&mut self, amount: u32) {
        self.amount = self.amount.saturating_add(amount);
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
    pub day_origin: Option<UnixMillis>,
}

pub type PlayerDefId = ContentId<PlayerDef>;

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct PlayerDef {
    pub model: ModelId,
    pub class: ClassId,
    pub stats: Stats,
    #[schemars(extend("x-usage" = "equipped at creation"))]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub equipped: Vec<ItemId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modules: Vec<PlayerModule>,
}

impl ContentFile for PlayerDef {
    const TABLE: &'static str = "player";

    fn fresh(near: &Nearby) -> Result<PlayerDef, String> {
        Ok(PlayerDef {
            model: near.file()?,
            class: near.file()?,
            stats: Stats::default(),
            equipped: Vec::new(),
            modules: Vec::new(),
        })
    }
}

impl ModuleSet for PlayerDef {
    type Module = PlayerModule;

    fn modules(&self) -> &[PlayerModule] {
        &self.modules
    }

    fn modules_mut(&mut self) -> &mut Vec<PlayerModule> {
        &mut self.modules
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

/// Answers every request of a kind for the player who sent it, telling them what refused one;
/// `commanding` requests are dropped while the player may not command their character.
pub(crate) fn player_requests<M: Message>(
    world: &mut World,
    commanding: bool,
    mut answer: impl FnMut(&mut World, Entity, M) -> Result<(), String>,
) {
    for request in drain_requests::<M>(world) {
        let Some(player) = sender_player(world, request.client_id) else {
            continue;
        };
        if commanding && !may_command(world, player) {
            continue;
        }
        if let Err(reason) = answer(world, player, request.message) {
            notification::error(world, player, reason);
        }
    }
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
        let day_origin = world
            .get_resource::<DayOrigin>()
            .and_then(|origin| origin.0);
        visibility::send_to(
            world,
            conn,
            Welcome {
                id,
                now,
                day_origin,
            },
        );
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

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
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
    let policy = world.resource::<SpawnRules>().policy;
    let requests = drain_requests::<JoinRequest>(world);
    if requests.is_empty() {
        return;
    }
    let area = area::load(&content, zone);
    for request in requests {
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
                    movement::place_beside(world, zone, &area.mesh, start.at),
                    snapshot::materialize(&content, &snapshot, start, clock),
                )
            }
            _ => {
                let name = world
                    .get::<Identity>(client_entity)
                    .map_or_else(|| format!("player {}", client.0), |id| id.name.clone());
                let spawn = spawn_position(world, policy, zone, area);
                let at = movement::place_beside(world, zone, &area.mesh, spawn);
                let fresh = CharacterSnapshot::of_player(&content, &name, def(&content));
                let start = Place { area: zone, at };
                let state = snapshot::materialize(&content, &fresh, start, clock);
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

fn spawn_position(
    world: &mut World,
    policy: SpawnPolicy,
    zone: AreaId,
    area: &area::Area,
) -> Pos<Meters> {
    let spawn = world.resource::<GameSettings>().initial_spawn;
    let fallback = || area.fallback_spawn();
    match policy {
        SpawnPolicy::Map if spawn.area == zone => spawn.at,
        SpawnPolicy::Map => fallback(),
        SpawnPolicy::Dist => area
            .mesh
            .random_point(&mut world.resource_mut::<Rng>(), |_| true)
            .unwrap_or_else(fallback),
    }
}

#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct SpawnPoint(pub Pos<Meters>);

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
    pub hotbar: Hotbar,
    pub basic_attack: BasicAttack,
    pub memory: Memory,
    pub heard: Heard,
    pub ledger: ShopLedger,
    pub quests: QuestLog,
    pub history: History,
}

impl CharacterState {
    pub fn of(world: &World, entity: Entity, spawn: Pos<Meters>) -> Option<CharacterState> {
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

pub(crate) fn place(
    world: &mut World,
    client: ClientId,
    zone: AreaId,
    at: Pos<Meters>,
    state: CharacterState,
) -> Entity {
    let content = world.resource::<Content>().clone();
    let def = def(&content);
    let model = def.model;
    let now = Seconds(world.resource::<Time>().elapsed_secs());
    let entity = world
        .spawn((
            Character::new(
                Position::at(at),
                Actor {
                    color: def.tint(),
                    facing: Radians::SOUTH,
                    action: Action::Idle,
                    pose: Pose::Idle,
                    model,
                    attack_rate: PlaybackRate::default(),
                    speed: MetersPerSec::default(),
                },
                Hitbox(ActorModel::get(&content, model).hitbox()),
                AreaTag { area: zone },
            ),
            OwnedBy(client),
            PlayedBy { client },
            HeardMove::default(),
            ArrivalHold {
                until: now + ARRIVAL_PATIENCE,
            },
            state,
        ))
        .id();
    let belongings = belongings::of(world, entity);
    world.entity_mut(entity).insert(belongings);
    if world.resource::<SpawnRules>().immortal {
        world.entity_mut(entity).insert(Immortal);
    }
    world.resource_mut::<Players>().0.insert(client, entity);
    entity
}

pub fn respawn(world: &mut World) {
    let content = world.resource::<Content>().clone();
    player_requests::<RespawnRequest>(world, false, |world, player, _| {
        let spawn = world.get::<SpawnPoint>(player).map(|spawn| spawn.0);
        if let Some(spawn) = spawn
            && stat::is_dead(world, player)
        {
            if let Some(mut memory) = world.get_mut::<Memory>(player) {
                memory.leave_area(&content);
            }
            combat::revive(world, player, spawn);
        }
        Ok(())
    });
}
