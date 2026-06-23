pub mod auth;
pub mod transport;

use bevy::prelude::*;
use bevy::tasks::{IoTaskPool, Task, block_on, futures_lite::future};
use bevy_replicon::prelude::RepliconChannels;
use renet2::{ConnectionConfig, RenetClient};
use renet2_netcode::{ClientAuthentication, ClientSocket, ConnectToken, NetcodeClientTransport};
use world::protocol::channels::RenetChannelsExt;
use world::protocol::session::ClientSessionPlugin;

use crate::net::auth::Session;
use crate::net::transport::{Client, RepliconRenetClientPlugin, Transport};
use crate::platform::StartParams;

pub struct NetPlugin;

impl Plugin for NetPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((ClientSessionPlugin, RepliconRenetClientPlugin))
            .add_systems(Update, (poll_session, announce));
    }
}

#[derive(Resource)]
pub struct Announce {
    pub spectate: bool,
}

/// Present while a connect token is being fetched — i.e. a connection attempt is in flight.
#[derive(Resource)]
pub struct PendingSession {
    task: Task<Result<Vec<u8>, String>>,
    spectate: bool,
}

/// Begins joining: asks the game server for a connect token over the browser network (non-blocking),
/// finished by [`poll_session`] once it resolves.
pub fn open_session(world: &mut World, spectate: bool) {
    let Some(session) = world.get_resource::<Session>().cloned() else {
        error!("no access token; cannot open a session");
        return;
    };
    let game_server_url = world.resource::<StartParams>().game_server_url.clone();
    let task = IoTaskPool::get().spawn(fetch_token(game_server_url, session.authorization));
    world.insert_resource(PendingSession { task, spectate });
}

fn poll_session(world: &mut World) {
    let Some(mut pending) = world.remove_resource::<PendingSession>() else {
        return;
    };
    match block_on(future::poll_once(&mut pending.task)) {
        Some(Ok(token)) => {
            connect(world, &token);
            world.insert_resource(Announce {
                spectate: pending.spectate,
            });
        }
        Some(Err(error)) => error!("could not open a session: {error}"),
        None => world.insert_resource(pending),
    }
}

async fn fetch_token(game_server_url: String, authorization: String) -> Result<Vec<u8>, String> {
    crate::platform::fetch(&format!("{game_server_url}/session"), &authorization).await
}

fn connect(world: &mut World, token: &[u8]) {
    let channels = world.resource::<RepliconChannels>();
    let connection_config =
        ConnectionConfig::from_channels(channels.server_configs(), channels.client_configs());
    let connect_token =
        ConnectToken::read(&mut std::io::Cursor::new(token)).expect("read connect token");
    let server_url = world.resource::<StartParams>().game_server_ws_url.clone();
    let socket = crate::platform::client_socket(&server_url);
    let client = RenetClient::new(connection_config, socket.is_reliable());
    let transport = NetcodeClientTransport::new(
        crate::platform::now(),
        ClientAuthentication::Secure { connect_token },
        socket,
    )
    .expect("client transport");
    world.insert_resource(Client(client));
    world.insert_resource(Transport(transport));
    info!("netcode connection opened");
}

fn announce(world: &mut World) {
    if world.get_resource::<Announce>().is_none()
        || world::protocol::session::my_id(world).is_none()
    {
        return;
    }
    let spectate = world.resource::<Announce>().spectate;
    info!(
        "connection welcomed; announcing {}",
        if spectate { "spectate" } else { "join" }
    );
    if spectate {
        world::protocol::session::spectate(world, None);
    } else {
        world::protocol::session::join(world);
    }
    world.remove_resource::<Announce>();
}
