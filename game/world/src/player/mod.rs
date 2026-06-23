//! Players: the replicated [`Owner`]/[`Xp`] of a player character and the [`ClientId`] keying a
//! connection, the join/respawn/welcome messages, and the server systems that admit joins, retire
//! disconnected accounts, and respawn the dead. [`session`] is the client's view of all this.

pub mod session;

use bevy_app::App;
use bevy_ecs::component::Component;
use bevy_ecs::message::Message;
use serde::{Deserialize, Serialize};

#[cfg(feature = "systems")]
use crate::Character;
#[cfg(feature = "systems")]
use crate::account::identity::Identity;
#[cfg(feature = "systems")]
use crate::actor::{ACTION_IDLE, Actor, ActorModel, Hitbox, Name, Rgba, set_action};
#[cfg(feature = "systems")]
use crate::area::{self, AreaDef, AreaTag};
#[cfg(feature = "systems")]
use crate::combat::{Stats, Vitals, is_dead};
#[cfg(feature = "systems")]
use crate::core::math::{Direction, Pos};
#[cfg(feature = "systems")]
use crate::core::table::Id;
#[cfg(feature = "systems")]
use crate::core::tiling::{Tiles, TilesPerSec};
#[cfg(feature = "systems")]
use crate::core::time::{Millis, PlaybackRate};
#[cfg(feature = "systems")]
use crate::items::Inventory;
#[cfg(feature = "systems")]
use crate::movement::{Position, Speed, forget};
#[cfg(feature = "systems")]
use crate::spectate::Spectators;
#[cfg(feature = "systems")]
use crate::visibility::OwnedBy;
#[cfg(feature = "systems")]
use bevy_ecs::lifecycle::{Add, Remove};
#[cfg(feature = "systems")]
use bevy_ecs::message::Messages;
#[cfg(feature = "systems")]
use bevy_ecs::observer::On;
#[cfg(feature = "systems")]
use bevy_ecs::prelude::*;
#[cfg(feature = "systems")]
use bevy_replicon::prelude::{FromClient, Replicated, SendTargets, ToClients};
#[cfg(feature = "systems")]
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

#[cfg(feature = "systems")]
const PLAYER_MAX_HEALTH: f32 = 30.0;
#[cfg(feature = "systems")]
const PLAYER_SPEED: TilesPerSec = TilesPerSec(Tiles(4.0));
#[cfg(feature = "systems")]
const PLAYER_DAMAGE: f32 = 6.0;
#[cfg(feature = "systems")]
const PLAYER_ATTACK_SPEED: PlaybackRate = PlaybackRate(1.2);
#[cfg(feature = "systems")]
const PLAYER_ATTACK_DELAY: Millis = Millis(200.0);
#[cfg(feature = "systems")]
const PLAYER_RANGE: Tiles = Tiles(1.5);
#[cfg(feature = "systems")]
const PLAYER_TINT: Rgba = Rgba(0xFFFF_FFFF);
#[cfg(feature = "systems")]
const PLAYER_MODEL: &str = "adventurer";

#[cfg(feature = "systems")]
#[derive(Resource, Default)]
pub struct Players(pub HashMap<ClientId, Entity>);

#[cfg(feature = "systems")]
pub(crate) fn sender_player(
    world: &World,
    sender: bevy_replicon::prelude::ClientId,
) -> Option<Entity> {
    let client = world.get::<ClientId>(sender.entity()?)?;
    world.resource::<Players>().0.get(client).copied()
}

#[cfg(feature = "systems")]
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
#[cfg(feature = "systems")]
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

#[cfg(feature = "systems")]
pub fn join(world: &mut World) {
    let zone = world.resource::<crate::WorldArea>().0;
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

#[cfg(feature = "systems")]
fn spawn_player(
    world: &mut World,
    client: ClientId,
    zone: Id<AreaDef>,
    at: Pos<Tiles>,
    name: String,
) {
    let max = PLAYER_MAX_HEALTH;
    place(
        world,
        client,
        zone,
        at,
        name,
        Vitals { health: max, max },
        Inventory { items: Vec::new() },
        Xp { amount: 0 },
    );
}

/// Fresh joins pass starting state; portals pass carried state from the previous world.
#[cfg(feature = "systems")]
#[allow(clippy::too_many_arguments)]
pub(crate) fn place(
    world: &mut World,
    client: ClientId,
    zone: Id<AreaDef>,
    at: Pos<Tiles>,
    name: String,
    vitals: Vitals,
    inventory: Inventory,
    xp: Xp,
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
                    dir: Direction::S as u8,
                    action: ACTION_IDLE,
                    model,
                    attack_rate: PLAYER_ATTACK_SPEED,
                },
                hitbox: Hitbox {
                    size: model.get().hitbox(),
                },
                vitals,
                area: AreaTag { area: zone },
                stats: Stats {
                    damage: PLAYER_DAMAGE,
                    attack_speed: PLAYER_ATTACK_SPEED,
                    attack_delay: PLAYER_ATTACK_DELAY,
                    range: PLAYER_RANGE,
                },
                speed: Speed {
                    value: PLAYER_SPEED,
                },
            },
            OwnedBy(client),
            Owner { client },
            inventory,
            xp,
        ))
        .id();
    world.resource_mut::<Players>().0.insert(client, entity);
    entity
}

#[cfg(feature = "systems")]
pub fn respawn(world: &mut World) {
    let zone = world.resource::<crate::WorldArea>().0;
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
        if let Some(mut vitals) = world.get_mut::<Vitals>(entity) {
            vitals.health = vitals.max;
        }
        if let Some(mut position) = world.get_mut::<Position>(entity) {
            position.pos = spawn;
        }
        if let Some(mut tag) = world.get_mut::<AreaTag>(entity) {
            tag.area = zone;
        }
        if let Some(mut actor) = world.get_mut::<Actor>(entity) {
            set_action(&mut actor, ACTION_IDLE);
        }
        forget(world, entity);
    }
}
