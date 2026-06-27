pub mod session;

use bevy_app::App;
use bevy_ecs::component::Component;
use bevy_ecs::message::Message;
use serde::{Deserialize, Serialize};

use crate::core::math::{Direction, Pos};
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
    let spawn = area::area(zone).spawn;
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
        spawn_player(world, client, zone, spawn, name);
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

fn spawn_player(world: &mut World, client: ClientId, zone: area::Id, at: Pos<Tiles>, name: String) {
    place(
        world,
        client,
        zone,
        at,
        CharacterState {
            name,
            stats: player_stats(),
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

pub fn player_stats() -> Stats {
    Stats(vec![
        StatKind::Health.new(PLAYER_MAX_HEALTH),
        StatKind::MaxHealth.new(PLAYER_MAX_HEALTH),
        StatKind::Damage.new(PLAYER_DAMAGE),
        StatKind::AttackSpeed.new(PLAYER_ATTACK_SPEED.0),
        StatKind::AttackDelay.new(PLAYER_ATTACK_DELAY.0),
        StatKind::Range.new(PLAYER_RANGE.0),
        StatKind::MovementSpeed.new(PLAYER_SPEED.0.0),
    ])
}

pub(crate) fn place(
    world: &mut World,
    client: ClientId,
    zone: area::Id,
    at: Pos<Tiles>,
    state: CharacterState,
) -> Entity {
    let model = crate::systems::actor::model_id(PLAYER_MODEL);
    let entity = world
        .spawn((
            Character {
                replicated: Replicated,
                position: Position { pos: at },
                name: Name { name: state.name },
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
    let spawn = area::area(zone).spawn;
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
