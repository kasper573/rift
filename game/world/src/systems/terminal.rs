use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bevy_app::App;
use bevy_ecs::message::Message;
use bevy_ecs::observer::On;
use bevy_ecs::prelude::*;
use bevy_replicon::prelude::{ConnectedClient, SendTargets, ToClients};
use serde::{Deserialize, Serialize};
use strum::VariantArray;

use crate::data::terminal::Id;
use crate::systems::account::identity::Identity;
use crate::systems::account::role::Role;
use crate::systems::player::{ClientId, Players};

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;

    app.add_client_message::<TerminalInput>(Channel::Ordered)
        .add_server_message::<TerminalTabs>(Channel::Ordered)
        .add_server_message::<TerminalLine>(Channel::Ordered);
}

/// A terminal definition: one tab in the client's terminal window. Rows are declared in
/// [`crate::data::terminal`].
pub struct Terminal {
    pub title: &'static str,
    pub access: Option<Role>,
}

impl Terminal {
    pub fn allows(&self, identity: Option<&Identity>) -> bool {
        match self.access {
            None => true,
            Some(role) => identity.is_some_and(|identity| identity.has_role(role)),
        }
    }
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TerminalInput {
    pub terminal: Id,
    pub text: String,
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TerminalTabs {
    pub tabs: Vec<Id>,
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TerminalLine {
    pub terminal: Id,
    pub text: String,
}

/// A validated input line, available to all terminal integrations for one tick via
/// [`TerminalInbox`]. An integration that acts on an entry exclusively (e.g. command dispatch)
/// calls [`TerminalEntry::consume`] so fallback integrations (e.g. chat) skip it; integrations
/// that observe everything (e.g. a log) simply ignore the flag.
#[derive(Debug)]
pub struct TerminalEntry {
    pub terminal: Id,
    pub text: String,
    pub conn: Entity,
    pub client: ClientId,
    pub player: Option<Entity>,
    consumed: AtomicBool,
}

impl TerminalEntry {
    pub fn consume(&self) {
        self.consumed.store(true, Ordering::Relaxed);
    }

    pub fn consumed(&self) -> bool {
        self.consumed.load(Ordering::Relaxed)
    }
}

/// This tick's validated input entries. Shared via [`Arc`] so integrations can iterate it while
/// mutating the world.
#[derive(Resource, Default)]
pub struct TerminalInbox(pub Arc<Vec<TerminalEntry>>);

/// Sends `text` to every connection that has access to `terminal`. Multi-line text becomes one
/// [`TerminalLine`] per line.
pub fn broadcast(world: &mut World, terminal: Id, text: impl Into<String>) {
    send(world, None, terminal, &text.into());
}

/// Sends `text` to one connection only — still access-checked against `terminal`.
pub fn reply(world: &mut World, conn: Entity, terminal: Id, text: impl Into<String>) {
    send(world, Some(conn), terminal, &text.into());
}

pub fn issue_tabs(
    add: On<Add, ClientId>,
    conns: Query<Option<&Identity>, With<ClientId>>,
    mut tabs: MessageWriter<ToClients<TerminalTabs>>,
) {
    let Ok(identity) = conns.get(add.entity) else {
        return;
    };
    let tabs_for_conn: Vec<Id> = Id::VARIANTS
        .iter()
        .copied()
        .filter(|id| id.get().allows(identity))
        .collect();
    tabs.write(ToClients {
        targets: SendTargets::Single(bevy_replicon::prelude::ClientId::Client(add.entity)),
        message: TerminalTabs {
            tabs: tabs_for_conn,
        },
    });
}

const MAX_INPUT_LEN: usize = 256;

pub fn ingest(world: &mut World) {
    let mut entries = Vec::new();
    for request in crate::systems::requests::<TerminalInput>(world) {
        let Some(conn) = request.client_id.entity() else {
            continue;
        };
        let Some(&client) = world.get::<ClientId>(conn) else {
            continue;
        };
        if !request
            .message
            .terminal
            .get()
            .allows(world.get::<Identity>(conn))
        {
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
            client,
            player: world.resource::<Players>().0.get(&client).copied(),
            consumed: AtomicBool::new(false),
        });
    }
    let mut inbox = world.resource_mut::<TerminalInbox>();
    if !entries.is_empty() || !inbox.0.is_empty() {
        inbox.0 = Arc::new(entries);
    }
}

fn send(world: &mut World, only: Option<Entity>, terminal: Id, text: &str) {
    let def = terminal.get();
    let recipients: Vec<Entity> = world
        .query_filtered::<(Entity, Option<&Identity>), (With<ClientId>, With<ConnectedClient>)>()
        .iter(world)
        .filter(|&(conn, identity)| only.is_none_or(|only| only == conn) && def.allows(identity))
        .map(|(conn, _)| conn)
        .collect();
    for conn in recipients {
        for line in text.lines() {
            world.write_message(ToClients {
                targets: SendTargets::Single(bevy_replicon::prelude::ClientId::Client(conn)),
                message: TerminalLine {
                    terminal,
                    text: line.to_owned(),
                },
            });
        }
    }
}
