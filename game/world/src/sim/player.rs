use std::collections::HashMap;

use bevy_ecs::lifecycle::{Add, Remove};
use bevy_ecs::message::Messages;
use bevy_ecs::observer::On;
use bevy_ecs::prelude::*;
use bevy_replicon::prelude::{FromClient, Replicated, SendTargets, ToClients};

use super::Character;
use super::combat::Stats;
use super::movement::{Speed, forget};
use super::spectate::Spectators;
use super::visibility::OwnedBy;
use crate::actors::ActorModel;
use crate::area::{self, AreaDef};
use crate::identity::Identity;
use crate::math::{Direction, Millis, PlaybackRate, Pos, Tiles, TilesPerSec};
use crate::protocol;
use crate::protocol::{
    ACTION_IDLE, Actor, AreaTag, ClientId, Hitbox, Inventory, JoinRequest, Name, Owner, Position,
    RespawnRequest, Rgba, Vitals, Welcome, Xp, is_dead, set_action,
};
use crate::table::Id;

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

/// The health players spawn (and respawn) with. The server may override it — e2e scenarios raise it
/// so a player can't die mid-test; without it, players use [`PLAYER_MAX_HEALTH`].
#[derive(Resource, Clone, Copy)]
pub struct PlayerHealth(pub f32);

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
    clients: Query<&ClientId>,
    mut players: ResMut<Players>,
    mut commands: Commands,
) {
    let Ok(id) = clients.get(remove.entity) else {
        return;
    };
    if let Some(entity) = players.0.remove(id) {
        commands.entity(entity).despawn();
    }
}

pub fn join(world: &mut World) {
    let zone = area::spawn_zone();
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
        spawn_player(world, client, client_entity, zone, spawn, name);
    }
}

fn spawn_player(
    world: &mut World,
    client: ClientId,
    client_entity: Entity,
    zone: Id<AreaDef>,
    at: Pos<Tiles>,
    name: String,
) {
    let model = Id::<ActorModel>::by_name(PLAYER_MODEL).expect("the player model exists");
    let max = world
        .get_resource::<PlayerHealth>()
        .map_or(PLAYER_MAX_HEALTH, |health| health.0);
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
                vitals: Vitals { health: max, max },
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
            OwnedBy(client_entity),
            Owner { client },
            Inventory { items: Vec::new() },
            Xp { amount: 0 },
        ))
        .id();
    world.resource_mut::<Players>().0.insert(client, entity);
}

pub fn respawn(world: &mut World) {
    let zone = area::spawn_zone();
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
        if let Some(mut actor) = world.get_mut::<protocol::Actor>(entity) {
            set_action(&mut actor, ACTION_IDLE);
        }
        forget(world, entity);
    }
}
