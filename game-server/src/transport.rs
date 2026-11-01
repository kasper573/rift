use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use engine::core::net::transport::{PROTOCOL_ID, connection_config, netcode_now};
use renet2::{ClientId, NetworkInfo, RenetServer, ServerEvent};
use renet2_netcode::{
    ConnectToken, NETCODE_KEY_BYTES, NativeSocket, NetcodeServerTransport, ServerAuthentication,
    ServerSetupConfig, generate_random_bytes,
};

/// Every client's connection: netcode over UDP, admitting only the clients that hold a connect
/// token this server minted, encrypting their packets and dropping those that fall silent.
///
/// One netcode server holds at most `SHARD_CLIENTS` clients, so the transport runs as many as
/// `max_clients` needs, each on a UDP port of its own, consecutive from the first port.
pub struct UdpTransport {
    shards: Vec<Shard>,
    /// Which shard each connected client plays through.
    clients: HashMap<ClientId, usize>,
    load: Load,
}

/// Mints the connect tokens the session endpoint hands out, each for one client of this server
/// process alone, as its key is drawn anew at every start, on the least loaded shard.
#[derive(Clone)]
pub struct TokenMinter {
    key: [u8; NETCODE_KEY_BYTES],
    shards: Vec<SocketAddr>,
    load: Load,
}

impl UdpTransport {
    pub fn bind(
        bind: SocketAddr,
        public_ip: IpAddr,
        max_clients: usize,
    ) -> (UdpTransport, TokenMinter) {
        let key = generate_random_bytes();
        let count = max_clients.div_ceil(SHARD_CLIENTS).max(1);
        let mut shards = Vec::with_capacity(count);
        let mut public = Vec::with_capacity(count);
        for index in 0..count {
            let port = match bind.port() {
                0 => 0,
                first => first + index as u16,
            };
            let address = SocketAddr::new(bind.ip(), port);
            let socket = UdpSocket::bind(address)
                .unwrap_or_else(|error| panic!("cannot bind {address}: {error}"));
            let local = socket.local_addr().expect("a bound udp address");
            let reached_at = SocketAddr::new(public_ip, local.port());
            let setup = ServerSetupConfig {
                current_time: netcode_now(),
                max_token_lifetime: TOKEN_LIFETIME.as_secs() as u16,
                max_clients: (max_clients - index * SHARD_CLIENTS).min(SHARD_CLIENTS),
                protocol_id: PROTOCOL_ID,
                socket_addresses: vec![vec![reached_at]],
                authentication: ServerAuthentication::Secure { private_key: key },
            };
            let socket = NativeSocket::new(socket).expect("a non-blocking udp socket");
            shards.push(Shard {
                server: RenetServer::new(connection_config()),
                transport: NetcodeServerTransport::new(setup, socket)
                    .expect("the netcode transport starts"),
                local,
            });
            public.push(reached_at);
        }
        let load = Load(Arc::new((0..count).map(|_| AtomicUsize::new(0)).collect()));
        let transport = UdpTransport {
            shards,
            clients: HashMap::new(),
            load: load.clone(),
        };
        let minter = TokenMinter {
            key,
            shards: public,
            load,
        };
        (transport, minter)
    }

    /// The port of the first shard, which the others follow when the first port was chosen.
    pub fn first_port(&self) -> u16 {
        self.shards[0].local.port()
    }

    /// Takes in what arrived since the last tick, and what connected and disconnected.
    pub fn update(&mut self, delta: Duration) -> Vec<ServerEvent> {
        let mut events = Vec::new();
        for (index, shard) in self.shards.iter_mut().enumerate() {
            shard.server.update(delta);
            if let Err(errors) = shard.transport.update(delta, &mut shard.server) {
                for error in errors {
                    eprintln!("udp transport: {error}");
                }
            }
            while let Some(event) = shard.server.get_event() {
                match event {
                    ServerEvent::ClientConnected { client_id } => {
                        self.clients.insert(client_id, index);
                    }
                    ServerEvent::ClientDisconnected { client_id, .. } => {
                        self.clients.remove(&client_id);
                    }
                }
                events.push(event);
            }
            self.load.0[index].store(shard.server.connected_clients(), Ordering::Relaxed);
        }
        events
    }

    pub fn receive(&mut self, client: ClientId, channel: u8) -> Option<Vec<u8>> {
        let message = self.server_of(client)?.receive_message(client, channel)?;
        Some(message.to_vec())
    }

    pub fn send(&mut self, client: ClientId, channel: u8, message: Vec<u8>) {
        if let Some(server) = self.server_of(client) {
            server.send_message(client, channel, message);
        }
    }

    pub fn disconnect(&mut self, client: ClientId) {
        if let Some(server) = self.server_of(client) {
            server.disconnect(client);
        }
    }

    pub fn network_info(&mut self, client: ClientId) -> Option<NetworkInfo> {
        self.server_of(client)?.network_info(client).ok()
    }

    pub fn disconnect_all(&mut self) {
        for shard in &mut self.shards {
            shard.transport.disconnect_all(&mut shard.server);
        }
    }

    pub fn send_packets(&mut self) {
        for shard in &mut self.shards {
            shard.transport.send_packets(&mut shard.server);
        }
    }

    fn server_of(&mut self, client: ClientId) -> Option<&mut RenetServer> {
        let shard = *self.clients.get(&client)?;
        Some(&mut self.shards[shard].server)
    }
}

impl TokenMinter {
    pub fn mint(&self, client_id: u64) -> ConnectToken {
        let shard = (0..self.shards.len())
            .min_by_key(|&index| self.load.0[index].load(Ordering::Relaxed))
            .expect("a transport has a shard");
        self.load.0[shard].fetch_add(1, Ordering::Relaxed);
        ConnectToken::generate(
            netcode_now(),
            PROTOCOL_ID,
            TOKEN_LIFETIME.as_secs(),
            client_id,
            SILENCE_TIMEOUT.as_secs() as i32,
            0,
            vec![self.shards[shard]],
            None,
            &self.key,
        )
        .expect("a connect token for one address")
    }
}

/// How long a session's connect token takes a client to use.
pub const TOKEN_LIFETIME: Duration = Duration::from_secs(30);

/// The most clients one netcode server holds.
const SHARD_CLIENTS: usize = 1024;

/// How long a connection may hear nothing before it ends: a client that vanished without a word
/// (a crash, a network switch) must not leave its character standing in the world for long.
const SILENCE_TIMEOUT: Duration = Duration::from_secs(15);

struct Shard {
    server: RenetServer,
    transport: NetcodeServerTransport,
    local: SocketAddr,
}

/// How many clients each shard holds, as of its last update and the tokens minted since, so
/// tokens spread over the shards.
#[derive(Clone)]
struct Load(Arc<Vec<AtomicUsize>>);
