pub mod session;

use bevy_app::App;
use bevy_ecs::component::Component;
use bevy_ecs::message::Message;
use serde::{Deserialize, Serialize};

use crate::core::assets::AssetService;
use crate::core::math::{Direction, Pos, Rng};
use crate::core::tiling::{Tiles, TilesPerSec};
use crate::core::time::{Millis, PlaybackRate};
use crate::systems::Character;
use crate::systems::account::identity::Identity;
use crate::systems::actor::{Action, Actor, Hitbox, Name, Rgba, set_action};
use crate::systems::area::{self, AreaTag};
use crate::systems::effect::TimedEffects;
use crate::systems::equipment::Equipment;
use crate::systems::item::Inventory;
use crate::systems::job::{self, Job};
use crate::systems::movement::{Position, forget};
use crate::systems::spectate::Spectators;
use crate::systems::stat::{self, StatKind, Stats};
use crate::systems::visibility::OwnedBy;
use bevy_ecs::lifecycle::{Add, Remove};
use bevy_ecs::observer::On;
use bevy_ecs::prelude::*;
use bevy_replicon::prelude::{Replicated, SendTargets, ToClients};
use std::collections::HashMap;

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;

    app.replicate::<Owner>()
        .replicate::<Xp>()
        .add_client_message::<JoinRequest>(Channel::Ordered)
        .add_client_message::<RespawnRequest>(Channel::Ordered)
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

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct JoinRequest;

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RespawnRequest;

#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Welcome {
    pub id: ClientId,
}

const PLAYER_MAX_HEALTH: f32 = 30.0;
/// Health a player spawns with when [`Immortal`] is set — high enough that nothing in the world can
/// grind it down, so test/dev sessions never die to NPCs.
const IMMORTAL_HEALTH: f32 = 9999.0;
const PLAYER_SPEED: TilesPerSec = TilesPerSec(Tiles(4.0));
const PLAYER_DAMAGE: f32 = 6.0;
const PLAYER_ATTACK_SPEED: PlaybackRate = PlaybackRate(1.2);
const PLAYER_ATTACK_DELAY: Millis = Millis(200.0);
const PLAYER_RANGE: Tiles = Tiles(1.5);
const PLAYER_TINT: Rgba = Rgba(0xFFFF_FFFF);
const PLAYER_MODEL: crate::data::model::Id = crate::data::model::Id::Adventurer;

#[derive(Resource, Default)]
pub struct Players(pub HashMap<ClientId, Entity>);

pub(crate) fn sender_player(
    world: &World,
    sender: bevy_replicon::prelude::ClientId,
) -> Option<Entity> {
    let client = world.get::<ClientId>(sender.entity()?)?;
    world.resource::<Players>().0.get(client).copied()
}

pub fn greet(
    add: On<Add, ClientId>,
    clients: Query<&ClientId>,
    mut welcome: MessageWriter<ToClients<Welcome>>,
) {
    let Ok(&id) = clients.get(add.entity) else {
        return;
    };
    welcome.write(ToClients {
        targets: SendTargets::Single(bevy_replicon::prelude::ClientId::Client(add.entity)),
        message: Welcome { id },
    });
}

pub fn client_left(
    remove: On<Remove, ClientId>,
    clients: Query<(Entity, &ClientId)>,
    mut players: ResMut<Players>,
    mut commands: Commands,
) {
    let Ok((_, id)) = clients.get(remove.entity) else {
        return;
    };
    if clients
        .iter()
        .any(|(entity, other)| entity != remove.entity && other == id)
    {
        return;
    }
    if let Some(entity) = players.0.remove(id) {
        commands.entity(entity).despawn();
    }
}

#[derive(Resource, Clone, Copy, Default, PartialEq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SpawnPolicy {
    #[default]
    Map,
    Dist,
}

/// When set, players spawn with [`IMMORTAL_HEALTH`] instead of [`PLAYER_MAX_HEALTH`] — a config toggle
/// (off in production) so test/dev sessions aren't killed by NPCs.
#[derive(Resource, Clone, Copy, Default)]
pub struct Immortal(pub bool);

pub fn join(world: &mut World) {
    let zone = world.resource::<crate::systems::WorldArea>().0;
    let assets = world.resource::<AssetService>().clone();
    let area = assets.resolve(zone.get().map, area::build_area);
    let policy = world
        .get_resource::<SpawnPolicy>()
        .copied()
        .unwrap_or_default();
    let immortal = world
        .get_resource::<Immortal>()
        .copied()
        .unwrap_or_default();
    for request in crate::systems::requests::<JoinRequest>(world) {
        let Some(client_entity) = request.client_id.entity() else {
            continue;
        };
        let Some(&client) = world.get::<ClientId>(client_entity) else {
            continue;
        };
        let playing = world.resource::<Players>().0.contains_key(&client);
        let spectating = world.resource::<Spectators>().0.contains_key(&client);
        if playing || spectating {
            continue;
        }
        let name = world
            .get::<Identity>(client_entity)
            .map_or_else(|| format!("player {}", client.0), |id| id.name.clone());
        let at = spawn_position(world, policy, area);
        spawn_player(world, client, zone, at, name, immortal);
    }
}

fn spawn_position(world: &mut World, policy: SpawnPolicy, area: &area::Area) -> Pos<Tiles> {
    match policy {
        SpawnPolicy::Map => area.spawn,
        SpawnPolicy::Dist if !area.walkable_nodes.is_empty() => {
            let index = world
                .resource_mut::<Rng>()
                .rand_range(0..area.walkable_nodes.len() as u32);
            area.walkable_nodes[index as usize]
        }
        SpawnPolicy::Dist => area.spawn,
    }
}

pub struct CharacterState {
    pub name: String,
    pub stats: Stats,
    pub inventory: Inventory,
    pub xp: Xp,
    pub equipment: Equipment,
    pub job: Job,
    pub timed: TimedEffects,
}

fn spawn_player(
    world: &mut World,
    client: ClientId,
    zone: area::Id,
    at: Pos<Tiles>,
    name: String,
    immortal: Immortal,
) {
    place(
        world,
        client,
        zone,
        at,
        CharacterState {
            name,
            stats: player_stats(immortal),
            inventory: Inventory::empty(),
            xp: Xp { amount: 0 },
            equipment: Equipment::default(),
            job: Job {
                def: job::default_job(),
            },
            timed: TimedEffects::default(),
        },
    );
}

pub fn player_stats(immortal: Immortal) -> Stats {
    let max_health = if immortal.0 {
        IMMORTAL_HEALTH
    } else {
        PLAYER_MAX_HEALTH
    };
    Stats(vec![
        StatKind::Health.of(max_health),
        StatKind::MaxHealth.of(max_health),
        StatKind::Damage.of(PLAYER_DAMAGE),
        StatKind::AttackSpeed.of(PLAYER_ATTACK_SPEED.0),
        StatKind::AttackDelay.of(PLAYER_ATTACK_DELAY.0),
        StatKind::Range.of(PLAYER_RANGE.0),
        StatKind::MovementSpeed.of(PLAYER_SPEED.0.0),
    ])
}

pub(crate) fn place(
    world: &mut World,
    client: ClientId,
    zone: area::Id,
    at: Pos<Tiles>,
    state: CharacterState,
) -> Entity {
    let model = PLAYER_MODEL;
    let assets = world.resource::<AssetService>().clone();
    let entity = world
        .spawn((
            Character {
                replicated: Replicated,
                position: Position { pos: at },
                actor: Actor {
                    color: PLAYER_TINT,
                    dir: Direction::S,
                    action: Action::Idle,
                    model,
                    attack_rate: PLAYER_ATTACK_SPEED,
                },
                hitbox: Hitbox {
                    size: assets
                        .resolve(*model.get(), crate::systems::actor::build_model)
                        .hitbox(),
                },
                area: AreaTag { area: zone },
            },
            Name { name: state.name },
            OwnedBy(client),
            Owner { client },
            state.inventory,
            state.xp,
            state.equipment,
            state.job,
            state.timed,
        ))
        .id();
    state.stats.apply(world, entity);
    world.resource_mut::<Players>().0.insert(client, entity);
    entity
}

pub fn respawn(world: &mut World) {
    let zone = world.resource::<crate::systems::WorldArea>().0;
    let assets = world.resource::<AssetService>().clone();
    let spawn = assets.resolve(zone.get().map, area::build_area).spawn;
    for request in crate::systems::requests::<RespawnRequest>(world) {
        let Some(entity) = sender_player(world, request.client_id) else {
            continue;
        };
        if !stat::is_dead(world, entity) {
            continue;
        }
        stat::refill(world, entity);
        if let Some(mut position) = world.get_mut::<Position>(entity) {
            position.pos = spawn;
        }
        if let Some(mut tag) = world.get_mut::<AreaTag>(entity) {
            tag.area = zone;
        }
        if let Some(mut actor) = world.get_mut::<Actor>(entity) {
            set_action(&mut actor, Action::Idle);
        }
        forget(world, entity);
    }
}
