pub mod auth;
pub mod transport;

use bevy::prelude::*;
use bevy::tasks::{IoTaskPool, Task, block_on, futures_lite::future};
use bevy_replicon::prelude::RepliconChannels;
use renet2::{ConnectionConfig, RenetClient};
use world::core::channels::RenetChannelsExt;

use crate::core::net::auth::Session;
use crate::core::net::transport::{Client, RepliconRenetClientPlugin, Socket};
use crate::core::platform::{StartParams, WsSocket};

pub struct NetPlugin;

impl Plugin for NetPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(RepliconRenetClientPlugin)
            .add_systems(Update, poll_session);
    }
}

#[derive(Resource)]
pub struct Announce {
    pub spectate: bool,
}

#[derive(Resource)]
pub struct PendingSession {
    task: Task<Result<String, String>>,
    spectate: bool,
}

pub fn open_session(world: &mut World, spectate: bool) {
    let Some(session) = world.get_resource::<Session>().cloned() else {
        error!("no access token; cannot open a session");
        return;
    };
    let game_server_url = world.resource::<StartParams>().game_server_url.clone();
    let task = IoTaskPool::get().spawn(fetch_ticket(game_server_url, session.authorization));
    world.insert_resource(PendingSession { task, spectate });
}

fn poll_session(world: &mut World) {
    let Some(mut pending) = world.remove_resource::<PendingSession>() else {
        return;
    };
    match block_on(future::poll_once(&mut pending.task)) {
        Some(Ok(ticket)) => {
            connect(world, ticket.trim());
            world.insert_resource(Announce {
                spectate: pending.spectate,
            });
        }
        Some(Err(error)) => error!("could not open a session: {error}"),
        None => world.insert_resource(pending),
    }
}

async fn fetch_ticket(game_server_url: String, authorization: String) -> Result<String, String> {
    crate::core::platform::fetch(&format!("{game_server_url}/session"), &authorization).await
}

fn connect(world: &mut World, ticket: &str) {
    let channels = world.resource::<RepliconChannels>();
    let connection_config =
        ConnectionConfig::from_channels(channels.server_configs(), channels.client_configs());
    let ws_url = world.resource::<StartParams>().game_server_ws_url.clone();
    // The websocket is reliable+ordered (TCP); renet handles channels and reliability over it.
    let client = RenetClient::new(connection_config, true);
    let socket = WsSocket::open(&format!("{ws_url}/?ticket={ticket}"));
    world.insert_resource(Client(client));
    world.insert_non_send(Socket(socket));
    info!("connection opened");
}
