pub mod bar;
pub mod errand;
pub mod roster;
pub mod session;
pub mod sheet;
pub mod snapshot;

use std::any::TypeId;
use std::collections::HashMap;

use bevy_app::App;
use bevy_ecs::bundle::Bundle;
use bevy_ecs::change_detection::Tick;
use bevy_ecs::lifecycle::HookContext;
use bevy_ecs::prelude::*;
use bevy_ecs::world::DeferredWorld;
use bevy_replication::Replicated;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use snapshot::{CharacterSnapshot, JoinAs, JoinOrigin, KeptAside, SavedCharacter};

use crate::core::content::{Content, ContentFile, ContentId, ModuleRule, ModuleSet, Nearby};
use crate::core::math::{Offset, Pos, Rect, Rng};
use crate::core::time::{self, DayClock, TimeCourse, UnixMillis, WallClock, WeatherClock};
use crate::core::units::{HourOfDay, Meters, MetersPerSec, PlaybackRate, Radians, Seconds};
use crate::systems::account::{AccountId, AccountIdentity};
use crate::systems::actor::{Action, Actor, ActorModel, Appearance, Hitbox, Name, Pose};
use crate::systems::area::transition::{self, Discovered};
use crate::systems::area::weather::ServerSky;
use crate::systems::area::{self, AreaId, AreaTag};
use crate::systems::class::{Class, ClassId};
use crate::systems::combat::{self, VitalRegen};
use crate::systems::crafting::RecipeMasteries;
use crate::systems::dialogue::{self, Heard};
use crate::systems::equipment::Equipment;
use crate::systems::history::History;
use crate::systems::item::Inventory;
use crate::systems::memory::Memory;
use crate::systems::mount::{self, RiderAct};
use crate::systems::movement::{self, HeardMove, Position, Spot};
use crate::systems::quest::QuestLog;
use crate::systems::script::{EventKind, Place};
use crate::systems::shop::{self, ShopLedger};
use crate::systems::skill::{BasicAttack, Cooldowns, Hotbar, Learned};
use crate::systems::stat::{self, Immortal, Stats};
use crate::systems::status::{self, CarriedStatuses, Statuses};
use crate::systems::visibility;
use crate::systems::{
    GameSettings, WorldArea, belongings, drain_requests, notification, reach, region, resource,
};

pub fn register(app: &mut App) {
    use bevy_replication::AppMessageExt;

    use bevy_replication::AppReplicationExt;

    app.replicate_once::<PlayedBy>()
        .replicate::<Away>()
        .replicate::<Xp>()
        .replicate_private::<CommandLock>()
        .replicate_private::<ArrivalHold>()
        .add_client_message::<JoinRequest>()
        .add_client_message::<RespawnRequest>()
        .add_client_message::<AreaDrawn>()
        .add_server_message::<Welcome>();
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
#[component(immutable, on_add = ClientId::connect, on_remove = ClientId::disconnect)]
pub struct ClientId(pub u32);

impl ClientId {
    fn connect(mut world: DeferredWorld, context: HookContext) {
        let Some(&client) = world.get::<ClientId>(context.entity) else {
            return;
        };
        if let Some(mut connected) = world.get_resource_mut::<Connected>() {
            connected.0.insert(client, context.entity);
        }
    }

    fn disconnect(mut world: DeferredWorld, context: HookContext) {
        let Some(&client) = world.get::<ClientId>(context.entity) else {
            return;
        };
        if let Some(mut connected) = world.get_resource_mut::<Connected>() {
            connected.0.remove(&client);
        }
    }
}

/// The clients connected to a server's world, playing a character or about to, each by its
/// connection: kept as connections come and go, so who is there is known without a look at the
/// world.
#[derive(Resource, Default)]
pub struct Connected(HashMap<ClientId, Entity>);

impl Connected {
    pub fn connection(&self, client: ClientId) -> Option<Entity> {
        self.0.get(&client).copied()
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PlayedBy {
    pub client: ClientId,
}

/// The account a character is played for. It stays with the character while no client plays it.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
#[component(immutable)]
pub struct CharacterAccount(pub AccountId);

/// A character staged from the game's content, standing in for the account's own: it is never
/// saved.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct StandIn;

/// A character whose player is gone. It stands in its world until `until`, as open to whatever
/// was after it as before, and does nothing of its own. A client of its account that connects
/// meanwhile plays it on.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Away {
    pub until: UnixMillis,
}

/// A character that left its world, for whoever runs the server's worlds to take: the client
/// that played it and, unless it stood in for the account's own, the account and the character
/// as it left.
pub struct Departure {
    pub client: ClientId,
    pub left: Option<SavedCharacter>,
}

/// The characters that left this world since the server's worlds last conferred.
#[derive(Resource, Default)]
pub struct Departures(pub Vec<Departure>);

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
    pub day: Option<TimeCourse<HourOfDay>>,
    pub weather: Option<TimeCourse<usize>>,
    pub clouds: Offset<Meters>,
}

pub type PlayerDefId = ContentId<PlayerDef>;

#[derive(JsonSchema, Clone, Serialize, Deserialize)]
pub struct PlayerDef {
    pub class: ClassId,
    pub stats: Stats,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modules: Vec<PlayerModule>,
}

impl ContentFile for PlayerDef {
    const TABLE: &'static str = "player";

    fn fresh(near: &Nearby) -> Result<PlayerDef, String> {
        Ok(PlayerDef {
            class: near.file()?,
            stats: Stats::default(),
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
    Regen(VitalRegen),
}

impl PlayerDef {
    pub fn regen(&self) -> Option<VitalRegen> {
        self.modules
            .iter()
            .map(|module| match module {
                PlayerModule::Regen(regen) => *regen,
            })
            .next()
    }
}

pub fn def(content: &Content) -> &PlayerDef {
    content.built::<GameSettings>().player.get(content)
}

/// How tall the game's player stands: the height of the body they are hit in.
pub fn stature(content: &Content) -> Meters {
    ActorModel::get(content, def(content).class.get(content).model)
        .hitbox()
        .height
}

#[derive(Resource, Default)]
pub struct Players(pub HashMap<ClientId, Entity>);

/// Whether any client is connected to this world, playing a character or about to.
pub fn any_connected(world: &World) -> bool {
    world
        .get_resource::<Connected>()
        .is_some_and(|connected| !connected.0.is_empty())
}

/// Whether anything of a player is in this world: a client, or a character, its player there or
/// gone.
pub fn any_present(world: &World) -> bool {
    any_connected(world)
        || world
            .get_resource::<Players>()
            .is_some_and(|players| !players.0.is_empty())
}

/// Whether a player is at their character: it is theirs to command, and is there for whatever
/// asks for a player's presence.
pub fn attended(world: &World, character: Entity) -> bool {
    world.get::<Away>(character).is_none()
}

pub fn commands_locked(world: &World, player: Entity) -> bool {
    world.get::<CommandLock>(player).is_some() || arriving(world, player)
}

pub fn may_command(world: &World, player: Entity) -> bool {
    !stat::is_dead(world, player) && !commands_locked(world, player)
}

pub fn arriving(world: &World, entity: Entity) -> bool {
    world.get::<ArrivalHold>(entity).is_some()
}

/// What a kind of request needs of the player who makes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestNeeds {
    /// Answered however the player's character fares.
    Nothing,
    /// Dropped while the player may not command their character.
    Command,
    /// A command a rider's seat must let them make: from a seat that does not, it is refused or
    /// made on foot, as their mount has it.
    Seat(RiderAct),
}

/// Answers every request of a kind for the player who sent it, telling them what refused one,
/// once the player meets what the kind `needs` of them.
pub(crate) fn player_requests<M: Message>(
    world: &mut World,
    needs: RequestNeeds,
    mut answer: impl FnMut(&mut World, Entity, M) -> Result<(), String>,
) {
    for request in drain_requests::<M>(world) {
        let Some(player) = conn_player(world, request.client) else {
            continue;
        };
        if needs != RequestNeeds::Nothing && !may_command(world, player) {
            continue;
        }
        let seated = match needs {
            RequestNeeds::Seat(act) => mount::seat::act(world, player, act),
            RequestNeeds::Nothing | RequestNeeds::Command => Ok(()),
        };
        let answered = seated.and_then(|()| answer(world, player, request.message));
        if let Err(reason) = answered {
            notification::error(world, player, reason);
        }
    }
}

/// Holds a player's character as arriving where it now stands, until its client reports the view
/// there drawn. Anyone else is left alone: only a player waits on a screen.
pub fn hold_arrival(world: &mut World, actor: Entity) {
    if world.get::<PlayedBy>(actor).is_none() {
        return;
    }
    let until = time::uptime(world) + ARRIVAL_PATIENCE;
    world.entity_mut(actor).insert(ArrivalHold { until });
}

pub fn arrival_holds(world: &mut World) {
    for request in drain_requests::<AreaDrawn>(world) {
        if let Some(player) = conn_player(world, request.client)
            && let Ok(mut player) = world.get_entity_mut(player)
        {
            player.remove::<ArrivalHold>();
        }
    }
    let now = time::uptime(world);
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

/// The character called `name` among those playing in this world, whatever its case.
pub fn named(world: &World, name: &str) -> Option<Entity> {
    world
        .resource::<Players>()
        .0
        .values()
        .copied()
        .find(|&character| {
            world
                .get::<Name>(character)
                .is_some_and(|called| called.name.eq_ignore_ascii_case(name))
        })
}

pub(crate) fn conn_player(world: &World, conn: Entity) -> Option<Entity> {
    let client = world.get::<ClientId>(conn)?;
    world.resource::<Players>().0.get(client).copied()
}

/// The connections owed a welcome. They are welcomed in the tick that follows rather than as they
/// come, so the greeting carries that tick's clock: a world nobody was connected to stood still,
/// and its own clock is as old as its last tick.
#[derive(Resource, Default)]
pub struct Unwelcomed(Vec<Entity>);

pub fn greet(add: On<Add<ClientId>>, mut owed: ResMut<Unwelcomed>) {
    owed.0.push(add.entity);
}

/// Has every client in the world told how its clocks run again, as its greeting did, once they
/// change.
pub fn tell_clocks(world: &mut World) {
    let conns: Vec<Entity> = world
        .query_filtered::<Entity, With<ClientId>>()
        .iter(world)
        .collect();
    world.resource_mut::<Unwelcomed>().0.extend(conns);
}

/// Welcomes every connection owed it: who it is to the server, the server's clock as this tick
/// has it, how the game's day and weather run, and how far the clouds have drifted over the area.
pub fn welcome(world: &mut World) {
    let owed = std::mem::take(&mut world.resource_mut::<Unwelcomed>().0);
    if owed.is_empty() {
        return;
    }
    let now = world.resource::<WallClock>().now;
    let day = world.get_resource::<DayClock>().and_then(|clock| clock.0);
    let weather = world
        .get_resource::<WeatherClock>()
        .and_then(|clock| clock.0);
    let drift = world.resource::<ServerSky>().drift();
    let clouds = Offset::new(drift.x, drift.y);
    for conn in owed {
        let Some(&id) = world.get::<ClientId>(conn) else {
            continue;
        };
        visibility::send_to(
            world,
            conn,
            Welcome {
                id,
                now,
                day,
                weather,
                clouds,
            },
        );
    }
}

/// As a client's last connection closes, its character stays behind for as long as the game has
/// characters linger.
pub fn client_left(
    remove: On<Remove<ClientId>>,
    clients: Query<(Entity, &ClientId)>,
    players: Res<Players>,
    mut commands: Commands,
) {
    if let Some(id) = last_connection(remove.entity, &clients)
        && let Some(&character) = players.0.get(&id)
    {
        commands.queue(move |world: &mut World| leave_behind(world, character));
    }
}

/// Keeps who is away as it is: a character no client plays is away, whichever way it lost its
/// player, one whose player is back is not, and one away for as long as the game has characters
/// linger leaves the world.
pub fn linger(world: &mut World, characters: &mut QueryState<(Entity, &PlayedBy, Option<&Away>)>) {
    let now = world.resource::<WallClock>().now;
    let connected = world.resource::<Connected>();
    let mut back = Vec::new();
    let mut left = Vec::new();
    let mut gone = Vec::new();
    for (character, played, away) in characters.iter(world) {
        match (connected.connection(played.client).is_some(), away) {
            (true, Some(_)) => back.push(character),
            (false, None) => left.push(character),
            (false, Some(away)) if away.until <= now => gone.push(character),
            _ => {}
        }
    }
    for character in back {
        world.entity_mut(character).remove::<Away>();
    }
    for character in left {
        leave_behind(world, character);
    }
    for character in gone {
        depart(world, character);
    }
}

/// Takes `character` out of the world, leaving it for whoever runs the server's worlds as it
/// stood.
pub fn depart(world: &mut World, character: Entity) {
    let Some(client) = world.get::<PlayedBy>(character).map(|played| played.client) else {
        return;
    };
    let left = SavedCharacter::of_live(world, character);
    world.resource_mut::<Players>().0.remove(&client);
    world.despawn(character);
    if let Some(mut departures) = world.get_resource_mut::<Departures>() {
        departures.0.push(Departure { client, left });
    }
}

/// Whether anything `character` is, or where it stands, changed since the world's change tick
/// was `since`.
pub fn changed_since(world: &World, character: Entity, since: Tick) -> bool {
    let Ok(character) = world.get_entity(character) else {
        return false;
    };
    let now = world.read_change_tick();
    let position = world.components().get_valid_id(TypeId::of::<Position>());
    CharacterState::get_component_ids(world.components())
        .chain([position])
        .flatten()
        .filter_map(|part| character.get_change_ticks_by_id(part))
        .any(|ticks| ticks.changed.is_newer_than(since, now))
}

/// Leaves `character` in the world without its player: whatever it was doing ends, and it is
/// away for as long as the game has characters linger, or gone at once where they don't.
fn leave_behind(world: &mut World, character: Entity) {
    if world.get_entity(character).is_err() {
        return;
    }
    let lingers_for = world.resource::<GameSettings>().lingers_for;
    if lingers_for <= Seconds(0.0) {
        depart(world, character);
        return;
    }
    movement::halt(world, character);
    dialogue::close_for(world, character);
    reach::forget(world, character);
    resource::stop(world, character);
    let until = world.resource::<WallClock>().now.after(lingers_for);
    world.entity_mut(character).insert(Away { until });
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
        let client_entity = request.client;
        let Some(&client) = world.get::<ClientId>(client_entity) else {
            continue;
        };
        if let Some(&character) = world.resource::<Players>().0.get(&client) {
            world.entity_mut(character).remove::<Away>();
            continue;
        }
        let clock = *world.resource::<WallClock>();
        let identity = world.get::<AccountIdentity>(client_entity).cloned();
        let (at, state, stand_in) = match world.get::<JoinAs>(client_entity) {
            Some(joining) if joining.start.area == zone => {
                let (snapshot, start) = (joining.snapshot.clone(), joining.start);
                let (taken, told, stand_in) = match joining.origin.clone() {
                    JoinOrigin::Staged => (clock.now, Vec::new(), true),
                    JoinOrigin::Saved { at, told } => (at, told, false),
                };
                region::hold_around(world, start.at);
                let mut state = snapshot::materialize(&content, &snapshot, start, clock, taken);
                state.kept_aside.told = told;
                (
                    movement::place_beside(world, zone, &area.mesh(), start.at),
                    state,
                    stand_in,
                )
            }
            _ => {
                let name = identity
                    .as_ref()
                    .map_or_else(|| format!("player {}", client.0), |id| id.name.clone());
                let spawn = spawn_position(world, policy, zone, area);
                region::hold_around(world, spawn);
                let at = movement::place_beside(world, zone, &area.mesh(), spawn);
                let fresh = CharacterSnapshot::of_player(&content, &name, def(&content));
                let start = Place { area: zone, at };
                let state = snapshot::materialize(&content, &fresh, start, clock, clock.now);
                (at, state, false)
            }
        };
        let played = Played {
            client,
            account: identity.map(|identity| identity.id),
            stand_in,
        };
        let entity = place(world, played, zone, at, state);
        spawned(world, entity);
    }
}

/// Whose a character is: the client that plays it, the account it is played for, and whether it
/// stands in for the account's own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Played {
    pub client: ClientId,
    pub account: Option<AccountId>,
    pub stand_in: bool,
}

impl Played {
    pub fn of(world: &World, character: Entity) -> Option<Played> {
        Some(Played {
            client: world.get::<PlayedBy>(character)?.client,
            account: world
                .get::<CharacterAccount>(character)
                .map(|account| account.0.clone()),
            stand_in: world.get::<StandIn>(character).is_some(),
        })
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
        SpawnPolicy::Dist => {
            let within = Rect::from_size(area.size);
            area.mesh()
                .random_point_within(&mut world.resource_mut::<Rng>(), within, |_| true)
                .unwrap_or_else(fallback)
        }
    }
}

#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct SpawnPoint(pub Pos<Meters>);

/// Everything a character is: what it crosses between worlds with, and what a snapshot of it
/// holds. A system that gives a character something more to keep lists it here, and it is
/// carried across a warp and saved by that line.
#[derive(Bundle, Debug, PartialEq)]
pub struct CharacterState {
    pub name: Name,
    pub appearance: Appearance,
    pub spawn: SpawnPoint,
    pub discovered: Discovered,
    pub stats: Stats,
    pub inventory: Inventory,
    pub xp: Xp,
    pub equipment: Equipment,
    pub class: Class,
    pub learned: Learned,
    pub masteries: RecipeMasteries,
    pub hotbar: Hotbar,
    pub basic_attack: BasicAttack,
    pub memory: Memory,
    pub heard: Heard,
    pub ledger: ShopLedger,
    pub quests: QuestLog,
    pub history: History,
    pub cooldowns: Cooldowns,
    pub statuses: Statuses,
    pub carried: CarriedStatuses,
    pub kept_aside: KeptAside,
}

impl CharacterState {
    pub fn of(world: &World, entity: Entity, spawn: Pos<Meters>) -> Option<CharacterState> {
        Some(CharacterState {
            name: world.get::<Name>(entity)?.clone(),
            appearance: world.get::<Appearance>(entity)?.clone(),
            spawn: SpawnPoint(spawn),
            discovered: world.get::<Discovered>(entity)?.clone(),
            stats: stat::snapshot(world, entity),
            inventory: world.get::<Inventory>(entity)?.clone(),
            xp: world.get::<Xp>(entity)?.clone(),
            equipment: world.get::<Equipment>(entity)?.clone(),
            class: *world.get::<Class>(entity)?,
            learned: world.get::<Learned>(entity).cloned().unwrap_or_default(),
            masteries: world
                .get::<RecipeMasteries>(entity)
                .cloned()
                .unwrap_or_default(),
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
            cooldowns: world.get::<Cooldowns>(entity).cloned().unwrap_or_default(),
            statuses: world.get::<Statuses>(entity).cloned().unwrap_or_default(),
            carried: status::aspect::carried(world, entity),
            kept_aside: world.get::<KeptAside>(entity).cloned().unwrap_or_default(),
        })
    }
}

pub(crate) fn place(
    world: &mut World,
    played: Played,
    zone: AreaId,
    at: Pos<Meters>,
    state: CharacterState,
) -> Entity {
    let client = played.client;
    let content = world.resource::<Content>().clone();
    let hitbox = Hitbox(ActorModel::get(&content, state.appearance.model).hitbox());
    let entity = world
        .spawn((
            Replicated,
            Position::at(at),
            Spot { area: zone, at },
            AreaTag { area: zone },
            Actor {
                facing: Radians::SOUTH,
                action: Action::Idle,
                pose: Pose::Idle,
                attack_rate: PlaybackRate::default(),
                speed: MetersPerSec::default(),
            },
            hitbox,
            PlayedBy { client },
            HeardMove::default(),
            state,
        ))
        .id();
    if let Some(account) = played.account {
        world.entity_mut(entity).insert(CharacterAccount(account));
    }
    if played.stand_in {
        world.entity_mut(entity).insert(StandIn);
    }
    hold_arrival(world, entity);
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
    player_requests::<RespawnRequest>(world, RequestNeeds::Nothing, |world, player, _| {
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
