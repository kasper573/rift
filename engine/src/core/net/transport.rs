//! How a client and its server carry their replication stream: one reliable ordered renet2
//! channel each way, over UDP, with netcode connecting, encrypting and timing out every
//! connection on the strength of a connect token the server's session endpoint hands out.

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::time::{Duration, SystemTime};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use bevy::prelude::*;
use bevy_replication::{ClientSystems, ServerLink};
use renet2::{ChannelConfig, ConnectionConfig, RenetClient, SendType};
use renet2_netcode::{ClientAuthentication, ConnectToken, NativeSocket, NetcodeClientTransport};

/// Names this game's protocol in every connect token; the session endpoint already refuses
/// clients of another build, so it tells nothing more apart.
pub const PROTOCOL_ID: u64 = u64::from_le_bytes(*b"rift\0\0\0\x01");

/// The one channel each way, which the whole stream travels.
pub const STREAM: u8 = 0;

pub fn connection_config() -> ConnectionConfig {
    let stream = || {
        vec![ChannelConfig {
            channel_id: STREAM,
            max_memory_usage_bytes: STREAM_MEMORY,
            send_type: SendType::ReliableOrdered {
                resend_time: RESEND_AFTER,
            },
        }]
    };
    ConnectionConfig::from_channels(stream(), stream())
}

/// A connect token as the session endpoint hands it out: its netcode bytes, in base64.
pub fn token_text(token: &ConnectToken) -> String {
    let mut bytes = Vec::new();
    token
        .write(&mut bytes)
        .expect("a connect token writes to memory");
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn read_token(text: &str) -> Option<ConnectToken> {
    let bytes = URL_SAFE_NO_PAD.decode(text.trim()).ok()?;
    ConnectToken::read(&mut bytes.as_slice()).ok()
}

/// The time netcode reckons connect tokens by, on server and client alike.
pub fn netcode_now() -> Duration {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("the clock is past 1970")
}

/// The client's connection to its server, from the moment it holds a connect token until the
/// connection ends. While it is connected, the replication stream runs through its `ServerLink`.
#[derive(Resource)]
pub struct Connection {
    client: RenetClient,
    transport: NetcodeClientTransport,
}

impl Connection {
    pub fn open(token: ConnectToken) -> std::io::Result<Connection> {
        let any_port = match token.server_addresses.iter().flatten().next() {
            Some(SocketAddr::V6(_)) => SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0)),
            _ => SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)),
        };
        let socket =
            NativeSocket::new(UdpSocket::bind(any_port)?).map_err(std::io::Error::other)?;
        let transport = NetcodeClientTransport::new(
            netcode_now(),
            ClientAuthentication::Secure {
                connect_token: token,
            },
            socket,
        )
        .map_err(std::io::Error::other)?;
        Ok(Connection {
            client: RenetClient::new(connection_config(), false),
            transport,
        })
    }

    pub fn is_connected(&self) -> bool {
        self.client.is_connected()
    }

    pub fn is_connecting(&self) -> bool {
        self.client.is_connecting()
    }
}

/// A connection let go of says goodbye, so the server lets its player go at once rather than
/// after the connection falls silent.
impl Drop for Connection {
    fn drop(&mut self) {
        self.transport.disconnect();
    }
}

/// Carries the replication stream between the `Connection` and the `ServerLink`, which stands
/// while the connection is up; a connection that ends takes both with it.
pub struct ClientTransportPlugin;

impl Plugin for ClientTransportPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PreUpdate, receive.before(ClientSystems::Receive))
            .add_systems(PostUpdate, send.after(ClientSystems::Send));
    }
}

fn receive(world: &mut World) {
    let delta = world.resource::<Time<Real>>().delta();
    let broken = world
        .get_resource::<ServerLink>()
        .is_some_and(ServerLink::broken);
    let Some(mut connection) = world.get_resource_mut::<Connection>() else {
        return;
    };
    let Connection { client, transport } = &mut *connection;
    if broken {
        client.disconnect();
    }
    client.update(delta);
    let updated = transport.update(delta, client);
    if client.is_disconnected() {
        if let Err(error) = updated {
            info!("connection ended: {error}");
        }
        world.remove_resource::<Connection>();
        world.remove_resource::<ServerLink>();
        return;
    }
    if !client.is_connected() {
        return;
    }
    let mut received = Vec::new();
    while let Some(bytes) = client.receive_message(STREAM) {
        received.push(bytes.to_vec());
    }
    let mut link = world.get_resource_or_init::<ServerLink>();
    for bytes in received {
        link.receive(bytes);
    }
}

fn send(world: &mut World) {
    let outgoing = world
        .get_resource_mut::<ServerLink>()
        .map(|mut link| link.take_outgoing())
        .unwrap_or_default();
    let Some(mut connection) = world.get_resource_mut::<Connection>() else {
        return;
    };
    let Connection { client, transport } = &mut *connection;
    for bytes in outgoing {
        client.send_message(STREAM, bytes);
    }
    if let Err(error) = transport.send_packets(client) {
        info!("connection ended: {error}");
    }
}

/// How much of the stream may wait unacknowledged before the connection gives up: far more than
/// a client is ever sent at once, as a player arriving among a crowd is.
const STREAM_MEMORY: usize = 32 * 1024 * 1024;

const RESEND_AFTER: Duration = Duration::from_millis(200);
