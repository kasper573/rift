use std::io::Cursor;
use std::net::{Ipv4Addr, UdpSocket};
use std::time::{SystemTime, UNIX_EPOCH};

use bevy::prelude::*;
use bevy_replicon::prelude::RepliconChannels;
use bevy_replicon_renet::netcode::{ClientAuthentication, ConnectToken, NetcodeClientTransport};
use bevy_replicon_renet::renet::ConnectionConfig;
use bevy_replicon_renet::{RenetChannelsExt, RenetClient, RepliconRenetPlugins};
use world::session::ClientSessionPlugin;

use crate::web;

pub struct NetPlugin;

impl Plugin for NetPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((ClientSessionPlugin, RepliconRenetPlugins))
            .add_systems(Update, announce);
    }
}

/// The intent to send once the connection is established — the server greets a fresh connection
/// with a [`Welcome`] before it accepts a join, so we wait for that rather than racing it.
#[derive(Resource)]
pub struct Announce {
    pub spectate: bool,
}

fn announce(world: &mut World) {
    if world.get_resource::<Announce>().is_none() || world::session::my_id(world).is_none() {
        return;
    }
    let spectate = world.resource::<Announce>().spectate;
    info!(
        "connection welcomed; announcing {}",
        if spectate { "spectate" } else { "join" }
    );
    if spectate {
        world::session::spectate(world, None);
    } else {
        world::session::join(world);
    }
    world.remove_resource::<Announce>();
}

/// Requests a session token from `{game_server_url}/session` with the given `Bearer <jwt>`
/// authorization.
pub fn request_token(game_server_url: &str, authorization: &str) -> Result<Vec<u8>, String> {
    let mut response = web::agent()
        .post(format!("{game_server_url}/session"))
        .header("Authorization", authorization)
        .send_empty()
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("session request failed: {}", response.status()));
    }
    response
        .body_mut()
        .read_to_vec()
        .map_err(|error| error.to_string())
}

/// Opens the netcode connection from a serialized [`ConnectToken`]; replicon begins replicating
/// once the renet client and transport are present.
pub fn connect(world: &mut World, token: &[u8]) {
    let channels = world.resource::<RepliconChannels>();
    let client = RenetClient::new(ConnectionConfig {
        server_channels_config: channels.server_configs(),
        client_channels_config: channels.client_configs(),
        ..default()
    });
    let connect_token = ConnectToken::read(&mut Cursor::new(token)).expect("read connect token");
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).expect("bind udp socket");
    let transport = NetcodeClientTransport::new(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock"),
        ClientAuthentication::Secure { connect_token },
        socket,
    )
    .expect("client transport");
    world.insert_resource(client);
    world.insert_resource(transport);
    info!("netcode connection opened");
}
