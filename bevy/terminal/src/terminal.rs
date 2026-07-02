use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bevy_app::{App, Update};
use bevy_ecs::message::{Message, Messages};
use bevy_ecs::prelude::*;
use bevy_replicon::prelude::{
    Channel, ClientId, ConnectedClient, FromClient, SendTargets, ToClients,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

pub trait TerminalKey:
    Copy + Eq + Ord + Hash + Serialize + DeserializeOwned + Send + Sync + 'static
{
}

impl<T: Copy + Eq + Ord + Hash + Serialize + DeserializeOwned + Send + Sync + 'static> TerminalKey
    for T
{
}

pub struct Terminal {
    pub access: Option<TerminalAccess>,
}

pub type TerminalAccess = fn(&World, Entity) -> bool;

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AvailableTerminals<K> {
    pub terminals: Vec<K>,
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TerminalInput<K> {
    pub terminal: K,
    pub text: String,
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TerminalLine<K> {
    pub terminal: K,
    pub text: String,
}

#[derive(Debug)]
pub struct TerminalEntry<K> {
    pub terminal: K,
    pub text: String,
    pub conn: Entity,
    consumed: AtomicBool,
}

impl<K> TerminalEntry<K> {
    pub fn consume(&self) {
        self.consumed.store(true, Ordering::Relaxed);
    }

    pub fn consumed(&self) -> bool {
        self.consumed.load(Ordering::Relaxed)
    }
}

#[derive(Resource)]
pub struct TerminalInbox<K>(pub Arc<Vec<TerminalEntry<K>>>);

impl<K> Default for TerminalInbox<K> {
    fn default() -> TerminalInbox<K> {
        TerminalInbox(Arc::default())
    }
}

pub fn register<K: TerminalKey>(app: &mut App, terminals: &HashMap<K, &'static Terminal>) {
    use bevy_replicon::prelude::{ClientMessageAppExt, ServerMessageAppExt};

    let mut table: Vec<(K, &'static Terminal)> = terminals
        .iter()
        .map(|(&key, &terminal)| (key, terminal))
        .collect();
    table.sort_unstable_by_key(|&(key, _)| key);
    app.insert_resource(TerminalTable(table))
        .init_resource::<TerminalInbox<K>>()
        .add_client_message::<TerminalInput<K>>(Channel::Ordered)
        .add_server_message::<TerminalLine<K>>(Channel::Ordered)
        .add_server_message::<AvailableTerminals<K>>(Channel::Ordered)
        .add_systems(Update, refresh::<K>);
}

pub fn broadcast<K: TerminalKey>(world: &mut World, terminal: K, text: impl Into<String>) {
    send(world, None, terminal, &text.into());
}

pub fn reply<K: TerminalKey>(
    world: &mut World,
    conn: Entity,
    terminal: K,
    text: impl Into<String>,
) {
    send(world, Some(conn), terminal, &text.into());
}

pub fn ingest<K: TerminalKey>(world: &mut World) {
    let requests: Vec<FromClient<TerminalInput<K>>> = world
        .resource_mut::<Messages<FromClient<TerminalInput<K>>>>()
        .drain()
        .collect();
    let mut entries = Vec::new();
    for request in requests {
        let Some(conn) = request.client_id.entity() else {
            continue;
        };
        if !open(world, request.message.terminal, conn) {
            continue;
        }
        let text: String = request
            .message
            .text
            .trim()
            .chars()
            .take(MAX_INPUT_LEN)
            .collect();
        if text.is_empty() {
            continue;
        }
        entries.push(TerminalEntry {
            terminal: request.message.terminal,
            text,
            conn,
            consumed: AtomicBool::new(false),
        });
    }
    let mut inbox = world.resource_mut::<TerminalInbox<K>>();
    if !entries.is_empty() || !inbox.0.is_empty() {
        inbox.0 = Arc::new(entries);
    }
}

#[derive(Resource)]
struct TerminalTable<K>(Vec<(K, &'static Terminal)>);

#[derive(Component)]
struct Issued<K>(Vec<K>);

const MAX_INPUT_LEN: usize = 256;

type Conns = QueryState<Entity, With<ConnectedClient>>;

fn refresh<K: TerminalKey>(world: &mut World, conns: &mut Conns) {
    let all: Vec<Entity> = conns.iter(world).collect();
    for conn in all {
        let shared: &World = world;
        let available: Vec<K> = shared
            .resource::<TerminalTable<K>>()
            .0
            .iter()
            .filter(|(_, terminal)| terminal.access.is_none_or(|access| access(shared, conn)))
            .map(|&(key, _)| key)
            .collect();
        if world
            .get::<Issued<K>>(conn)
            .is_some_and(|issued| issued.0 == available)
        {
            continue;
        }
        world.write_message(ToClients {
            targets: SendTargets::Single(ClientId::Client(conn)),
            message: AvailableTerminals {
                terminals: available.clone(),
            },
        });
        world.entity_mut(conn).insert(Issued(available));
    }
}

fn open<K: TerminalKey>(world: &World, terminal: K, conn: Entity) -> bool {
    world
        .resource::<TerminalTable<K>>()
        .0
        .iter()
        .find(|&&(key, _)| key == terminal)
        .is_some_and(|(_, terminal)| terminal.access.is_none_or(|access| access(world, conn)))
}

fn send<K: TerminalKey>(world: &mut World, only: Option<Entity>, terminal: K, text: &str) {
    let mut conns = world.query_filtered::<Entity, With<ConnectedClient>>();
    let shared: &World = world;
    let recipients: Vec<Entity> = conns
        .iter(shared)
        .filter(|&conn| only.is_none_or(|only| only == conn) && open(shared, terminal, conn))
        .collect();
    for conn in recipients {
        for line in text.lines() {
            world.write_message(ToClients {
                targets: SendTargets::Single(ClientId::Client(conn)),
                message: TerminalLine {
                    terminal,
                    text: line.to_owned(),
                },
            });
        }
    }
}
