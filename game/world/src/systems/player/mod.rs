//! Players: the replicated [`Owner`]/[`Xp`] of a player character and the [`ClientId`] keying a
//! connection, the join/respawn/welcome messages, and the server systems that admit joins, retire
//! disconnected accounts, and respawn the dead. [`session`] is the client's view of all this.

pub mod session;

use bevy_app::App;
use bevy_ecs::component::Component;
use bevy_ecs::message::Message;
use serde::{Deserialize, Serialize};

use crate::core::math::{Direction, Pos};
use crate::core::table::Id;
use crate::core::tiling::{Tiles, TilesPerSec};
use crate::core::time::{Millis, PlaybackRate};
use crate::systems::Character;
use crate::systems::account::identity::Identity;
use crate::systems::actor::{Action, Actor, ActorModel, Hitbox, Name, Rgba, set_action};
use crate::systems::area::{self, AreaDef, AreaTag};
use crate::systems::combat::is_dead;
use crate::systems::effect::TimedEffects;
use crate::systems::equipment::Equipment;
use crate::systems::items::Inventory;
use crate::systems::job::{self, Job};
use crate::systems::movement::{Position, forget};
use crate::systems::spectate::Spectators;
use crate::systems::stat::{
    self, AttackDelayStat, AttackSpeedStat, DamageStat, HealthStat, MaxHealthStat,
    MovementSpeedStat, RangeStat, StatSet,
};
use crate::systems::visibility::OwnedBy;
use bevy_ecs::lifecycle::{Add, Remove};
use bevy_ecs::message::Messages;
use bevy_ecs::observer::On;
use bevy_ecs::prelude::*;
use bevy_replicon::prelude::{FromClient, Replicated, SendTargets, ToClients};
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
const PLAYER_SPEED: TilesPerSec = TilesPerSec(Tiles(4.0));
const PLAYER_DAMAGE: f32 = 6.0;
const PLAYER_ATTACK_SPEED: PlaybackRate = PlaybackRate(1.2);
const PLAYER_ATTACK_DELAY: Millis = Millis(200.0);
const PLAYER_RANGE: Tiles = Tiles(1.5);
const PLAYER_TINT: Rgba = Rgba(0xFFFF_FFFF);
const PLAYER_MODEL: &str = "adventurer";

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

/// A character outlives any single connection: the same account can hold several sockets (one per
/// open tab). Only the departure of the last socket retires the character.
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

pub fn join(world: &mut World) {
    let zone = world.resource::<crate::systems::WorldArea>().0;
    let spawn = area::areas()[zone.index()].spawn;
    let requests: Vec<FromClient<JoinRequest>> = world
        .resource_mut::<Messages<FromClient<JoinRequest>>>()
        .drain()
        .collect();
    for request in requests {
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
        spawn_player(world, client, zone, spawn, name);
    }
}

fn spawn_player(
    world: &mut World,
    client: ClientId,
    zone: Id<AreaDef>,
    at: Pos<Tiles>,
    name: String,
) {
    place(
        world,
        client,
        zone,
        at,
        name,
        player_stats(),
        Inventory::empty(),
        Xp { amount: 0 },
        Equipment::default(),
        Job {
            def: job::default_job(),
        },
        TimedEffects::default(),
    );
}

/// The player's authored base stats — the single source of the player's numbers.
pub fn player_stats() -> StatSet {
    let mut stats = StatSet::default();
    stats.add(HealthStat.into(), PLAYER_MAX_HEALTH);
    stats.add(MaxHealthStat.into(), PLAYER_MAX_HEALTH);
    stats.add(DamageStat.into(), PLAYER_DAMAGE);
    stats.add(AttackSpeedStat.into(), PLAYER_ATTACK_SPEED.0);
    stats.add(AttackDelayStat.into(), PLAYER_ATTACK_DELAY.0);
    stats.add(RangeStat.into(), PLAYER_RANGE.0);
    stats.add(MovementSpeedStat.into(), PLAYER_SPEED.0.0);
    stats
}

/// Fresh joins pass starting state; portals pass carried state from the previous world.
#[allow(clippy::too_many_arguments)]
pub(crate) fn place(
    world: &mut World,
    client: ClientId,
    zone: Id<AreaDef>,
    at: Pos<Tiles>,
    name: String,
    stats: StatSet,
    inventory: Inventory,
    xp: Xp,
    equipment: Equipment,
    job: Job,
    timed: TimedEffects,
) -> Entity {
    let model = Id::<ActorModel>::by_name(PLAYER_MODEL).expect("the player model exists");
    let entity = world
        .spawn((
            Character {
                replicated: Replicated,
                position: Position { pos: at },
                name: Name { name },
                actor: Actor {
                    color: PLAYER_TINT,
                    dir: Direction::S,
                    action: Action::Idle,
                    model,
                    attack_rate: PLAYER_ATTACK_SPEED,
                },
                hitbox: Hitbox {
                    size: model.get().hitbox(),
                },
                area: AreaTag { area: zone },
            },
            OwnedBy(client),
            Owner { client },
            inventory,
            xp,
            equipment,
            job,
            timed,
        ))
        .id();
    stats.apply(world, entity);
    world.resource_mut::<Players>().0.insert(client, entity);
    entity
}

pub fn respawn(world: &mut World) {
    let zone = world.resource::<crate::systems::WorldArea>().0;
    let spawn = area::areas()[zone.index()].spawn;
    let requests: Vec<FromClient<RespawnRequest>> = world
        .resource_mut::<Messages<FromClient<RespawnRequest>>>()
        .drain()
        .collect();
    for request in requests {
        let Some(entity) = sender_player(world, request.client_id) else {
            continue;
        };
        if !is_dead(world, entity) {
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
