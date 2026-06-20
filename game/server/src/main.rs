mod auth;

use std::collections::HashMap;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_replicon::prelude::{ConnectedClient, RepliconChannels, ServerState};
use bevy_replicon::shared::backend::server_messages::ServerMessages;
use bevy_replicon_renet::RenetChannelsExt;
use bevy_replicon_renet::netcode::{
    ConnectToken, NETCODE_KEY_BYTES, NetcodeServerTransport, ServerAuthentication, ServerConfig,
};
use bevy_replicon_renet::renet::{ConnectionConfig, DisconnectReason, RenetServer, ServerEvent};
use bevy_state::prelude::NextState;
use metrics::{counter, gauge, histogram};
use rand::RngCore;
use world::area::{self, AreaDef};
use world::sim::transition;
use world::table::Id;
use world::{ClientId, Identity, TICK_HZ};

const PROTOCOL_ID: u64 = 0x0072_6966_7400_0001;
const TOKEN_EXPIRE: Duration = Duration::from_secs(30);
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_CLIENTS: usize = 256;

#[derive(serde::Deserialize)]
struct Config {
    port: u16,
    public_host: String,
    pyroscope_enabled: bool,
    pyroscope_sample_hz: u32,
    /// Optional override for the health players spawn with; e2e scenarios raise it so a player
    /// can't die mid-test. Absent in real deployments, where players use the in-game default.
    player_health: Option<f32>,
}

fn main() {
    world::assets::init(std::env::var_os("RIFT_ASSETS_DIR").expect("RIFT_ASSETS_DIR must be set"));
    world::sim::validate();
    let config: Config = envy::prefixed("RIFT_GAME_SERVER_")
        .from_env()
        .expect("RIFT_GAME_SERVER_* environment");
    let _profiler = service::profiler(
        "rift-game-server",
        config.pyroscope_enabled,
        config.pyroscope_sample_hz,
    );
    let bind: SocketAddr = format!("0.0.0.0:{}", config.port)
        .parse()
        .expect("server bind address");
    let public: SocketAddr = format!("{}:{}", config.public_host, config.port)
        .to_socket_addrs()
        .expect("resolve RIFT_GAME_SERVER_PUBLIC_HOST")
        .next()
        .expect("RIFT_GAME_SERVER_PUBLIC_HOST resolves to an address");

    let mut private_key = [0u8; NETCODE_KEY_BYTES];
    rand::rng().fill_bytes(&mut private_key);

    let sessions = Sessions::default();
    let http = Http {
        sessions: sessions.clone(),
        verifier: verifier(),
        prometheus: metrics_recorder(),
        private_key,
        public,
    };
    std::thread::spawn(move || serve_http(bind, http));

    simulate(bind, public, private_key, sessions, config.player_health);
}

fn simulate(
    bind: SocketAddr,
    public: SocketAddr,
    private_key: [u8; NETCODE_KEY_BYTES],
    sessions: Sessions,
    player_health: Option<f32>,
) {
    let spawn = area::spawn_zone().index();
    let mut worlds: Vec<App> = area::areas()
        .iter()
        .map(|a| build_world(a.id, player_health))
        .collect();

    let (connection_config, client_channels) = {
        let channels = worlds[0].world().resource::<RepliconChannels>();
        let config = ConnectionConfig {
            server_channels_config: channels.server_configs(),
            client_channels_config: channels.client_configs(),
            ..Default::default()
        };
        (config, channels.client_channels().len())
    };
    let mut server = RenetServer::new(connection_config);
    let socket =
        UdpSocket::bind(bind).unwrap_or_else(|error| panic!("cannot bind udp {bind}: {error}"));
    let mut transport = NetcodeServerTransport::new(
        ServerConfig {
            current_time: unix_now(),
            max_clients: MAX_CLIENTS,
            protocol_id: PROTOCOL_ID,
            public_addresses: vec![public],
            authentication: ServerAuthentication::Secure { private_key },
        },
        socket,
    )
    .expect("netcode server transport");

    println!(
        "mmo server listening: netcode udp {bind}, public {public} ({} area worlds)",
        worlds.len()
    );

    let mut conns: HashMap<u64, Conn> = HashMap::new();
    let mut transfers: Vec<Transfer> = Vec::new();
    let mut next_client = 1u32;
    let mut tick = 0u64;
    let frame = TICK_HZ.period();
    let mut last = Instant::now();
    loop {
        let started = Instant::now();
        let dt = started - last;
        last = started;
        tick += 1;

        server.update(dt);
        if let Err(error) = transport.update(dt, &mut server) {
            eprintln!("netcode transport update failed: {error}");
        }

        while let Some(event) = server.get_event() {
            match event {
                ServerEvent::ClientConnected { client_id } => {
                    let client = ClientId(next_client);
                    next_client += 1;
                    counter!("rift_client_connections_opened_total").increment(1);
                    let identity = sessions.take(client_id);
                    let entity = spawn_conn(&mut worlds[spawn], client, client_id, identity);
                    conns.insert(
                        client_id,
                        Conn {
                            area: spawn,
                            entity,
                            client,
                        },
                    );
                }
                ServerEvent::ClientDisconnected { client_id, reason } => {
                    counter!("rift_client_connections_closed_total", "code" => disconnect_code(&reason))
                        .increment(1);
                    if let Some(conn) = conns.remove(&client_id) {
                        worlds[conn.area].world_mut().despawn(conn.entity);
                    }
                }
            }
        }

        for (&network_id, conn) in &conns {
            let world = worlds[conn.area].world_mut();
            for channel in 0..client_channels as u8 {
                while let Some(message) = server.receive_message(network_id, channel) {
                    world.resource_mut::<ServerMessages>().insert_received(
                        conn.entity,
                        channel,
                        message,
                    );
                }
            }
        }

        for app in worlds.iter_mut() {
            app.update();
        }

        begin_transfers(&mut worlds, &conns, &mut transfers, tick);

        for app in worlds.iter_mut() {
            let world = app.world_mut();
            let sent: Vec<_> = world
                .resource_mut::<ServerMessages>()
                .drain_sent()
                .collect();
            for (entity, channel, message) in sent {
                if let Some(&Wire(network_id)) = world.get::<Wire>(entity) {
                    server.send_message(network_id, channel as u8, message);
                }
            }
        }

        finish_transfers(&mut worlds, &mut conns, &mut transfers, tick);

        transport.send_packets(&mut server);

        counter!("rift_ticks_total").increment(1);
        histogram!("rift_tick_duration_seconds").record(started.elapsed().as_secs_f64());
        gauge!("rift_clients_connected").set(conns.len() as f64);
        record_ecs_metrics(&worlds);
        record_network_metrics(&server, &conns);

        if let Some(remaining) = frame.checked_sub(started.elapsed()) {
            std::thread::sleep(remaining);
        }
    }
}

fn record_ecs_metrics(worlds: &[App]) {
    gauge!("rift_worlds").set(worlds.len() as f64);
    let mut entities = 0usize;
    let mut components = 0usize;
    let mut archetypes = 0usize;
    let mut tables = 0usize;
    for app in worlds {
        let world = app.world();
        for archetype in world.archetypes().iter() {
            entities += archetype.len() as usize;
            components += archetype.len() as usize * archetype.component_count();
        }
        archetypes += world.archetypes().len();
        tables += world.storages().tables.len();
    }
    gauge!("rift_entities").set(entities as f64);
    gauge!("rift_components").set(components as f64);
    gauge!("rift_archetypes").set(archetypes as f64);
    gauge!("rift_tables").set(tables as f64);
}

fn record_network_metrics(server: &RenetServer, conns: &HashMap<u64, Conn>) {
    let mut sent_per_sec = 0.0;
    let mut received_per_sec = 0.0;
    let mut max_packet_loss = 0.0;
    for &network_id in conns.keys() {
        sent_per_sec += server.bytes_sent_per_sec(network_id);
        received_per_sec += server.bytes_received_per_sec(network_id);
        max_packet_loss = f64::max(max_packet_loss, server.packet_loss(network_id));
        histogram!("rift_client_rtt_seconds").record(server.rtt(network_id));
    }
    gauge!("rift_net_bytes_sent_per_sec").set(sent_per_sec);
    gauge!("rift_net_bytes_received_per_sec").set(received_per_sec);
    gauge!("rift_packet_loss_ratio").set(max_packet_loss);
}

fn disconnect_code(reason: &DisconnectReason) -> &'static str {
    match reason {
        DisconnectReason::Transport => "transport",
        DisconnectReason::DisconnectedByClient => "by_client",
        DisconnectReason::DisconnectedByServer => "by_server",
        DisconnectReason::PacketSerialization(_) => "packet_serialization",
        DisconnectReason::PacketDeserialization(_) => "packet_deserialization",
        DisconnectReason::ReceivedInvalidChannelId(_) => "invalid_channel_id",
        DisconnectReason::SendChannelError { .. } => "send_channel_error",
        DisconnectReason::ReceiveChannelError { .. } => "receive_channel_error",
    }
}

struct Conn {
    area: usize,
    entity: Entity,
    client: ClientId,
}

#[derive(Component)]
struct Wire(u64);

fn build_world(area: Id<AreaDef>, player_health: Option<f32>) -> App {
    let mut app = world::sim::server_app(area);
    if let Some(health) = player_health {
        app.insert_resource(world::sim::player::PlayerHealth(health));
    }
    app.finish();
    app.cleanup();
    app.world_mut()
        .resource_mut::<NextState<ServerState>>()
        .set(ServerState::Running);
    app.update();
    app
}

fn spawn_conn(
    app: &mut App,
    client: ClientId,
    network_id: u64,
    identity: Option<Identity>,
) -> Entity {
    let world = app.world_mut();
    let mut entity = world.spawn((ConnectedClient { max_size: 1200 }, client, Wire(network_id)));
    if let Some(identity) = identity {
        entity.insert(identity);
    }
    entity.id()
}

/// A connection whose character left its world (despawned) and is waiting to be re-created in the
/// destination world. The one-tick wait lets the source world's despawns reach the client first, so
/// its replication state is empty before the destination's fresh snapshot arrives over the same
/// connection — no entity-id or tick collision between the two worlds.
struct Transfer {
    network_id: u64,
    traveler: transition::Traveler,
    departed_tick: u64,
}

/// Phase 1: despawn the character of everyone who stepped through a cross-area portal this tick (so
/// replicon despawns this world's entities for their client), and queue the connection to move.
fn begin_transfers(
    worlds: &mut [App],
    conns: &HashMap<u64, Conn>,
    transfers: &mut Vec<Transfer>,
    tick: u64,
) {
    for (area, app) in worlds.iter_mut().enumerate() {
        for traveler in transition::departing(app.world_mut()) {
            let Some((&network_id, _)) = conns
                .iter()
                .find(|(_, conn)| conn.area == area && conn.client == traveler.client)
            else {
                continue;
            };
            transfers.push(Transfer {
                network_id,
                traveler,
                departed_tick: tick,
            });
        }
    }
}

/// Phase 2: a tick after departing — once the source world's despawns have been sent — move the
/// connection to the destination world and re-create the character there.
fn finish_transfers(
    worlds: &mut [App],
    conns: &mut HashMap<u64, Conn>,
    transfers: &mut Vec<Transfer>,
    tick: u64,
) {
    let mut index = 0;
    while index < transfers.len() {
        if tick <= transfers[index].departed_tick {
            index += 1;
            continue;
        }
        let Transfer {
            network_id,
            traveler,
            ..
        } = transfers.remove(index);
        let Some(conn) = conns.remove(&network_id) else {
            continue;
        };
        let client = traveler.client;
        let dest = traveler.dest_area.index();
        let identity = worlds[conn.area]
            .world()
            .get::<Identity>(conn.entity)
            .cloned();
        worlds[conn.area].world_mut().despawn(conn.entity);
        let entity = spawn_conn(&mut worlds[dest], client, network_id, identity);
        transition::arrive(worlds[dest].world_mut(), entity, traveler);
        conns.insert(
            network_id,
            Conn {
                area: dest,
                entity,
                client,
            },
        );
    }
}

#[derive(Resource, Clone, Default)]
struct Sessions(Arc<Mutex<HashMap<u64, Pending>>>);

struct Pending {
    identity: Identity,
    minted: Instant,
}

impl Sessions {
    fn put(&self, client_id: u64, identity: Identity) {
        let mut map = self.0.lock().expect("sessions lock");
        let now = Instant::now();
        map.retain(|_, pending| {
            now.duration_since(pending.minted) < TOKEN_EXPIRE + Duration::from_secs(5)
        });
        map.insert(
            client_id,
            Pending {
                identity,
                minted: now,
            },
        );
    }

    fn take(&self, client_id: u64) -> Option<Identity> {
        self.0
            .lock()
            .expect("sessions lock")
            .remove(&client_id)
            .map(|pending| pending.identity)
    }
}

#[derive(Clone)]
struct Http {
    sessions: Sessions,
    verifier: Arc<Mutex<auth::Verifier>>,
    prometheus: metrics_exporter_prometheus::PrometheusHandle,
    private_key: [u8; NETCODE_KEY_BYTES],
    public: SocketAddr,
}

#[tokio::main(flavor = "current_thread")]
async fn serve_http(addr: SocketAddr, http: Http) {
    let router = axum::Router::new()
        .route("/session", post(session))
        .route("/health", get(health))
        .route("/metrics", get(scrape))
        .with_state(http);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|error| panic!("cannot bind {addr}: {error}"));
    println!("http listening on {addr}");
    ctrlc::set_handler(|| std::process::exit(0)).expect("install stop handler");
    axum::serve(listener, router).await.expect("axum serves");
}

async fn session(State(http): State<Http>, headers: HeaderMap) -> Response {
    let authorization = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let identity = match resolve(&http, authorization) {
        Ok(identity) => identity,
        Err(status) => return status.into_response(),
    };

    let client_id = rand::rng().next_u64();
    http.sessions.put(client_id, identity);
    let token = ConnectToken::generate(
        unix_now(),
        PROTOCOL_ID,
        TOKEN_EXPIRE.as_secs(),
        client_id,
        CONNECTION_TIMEOUT.as_secs() as i32,
        vec![http.public],
        None,
        &http.private_key,
    )
    .expect("connect token");
    let mut body = Vec::new();
    token.write(&mut body).expect("serialize connect token");
    (
        [
            (header::CONTENT_TYPE, "application/octet-stream"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        body,
    )
        .into_response()
}

fn resolve(http: &Http, authorization: &str) -> Result<Identity, StatusCode> {
    let token = authorization
        .strip_prefix("Bearer ")
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let claims = http
        .verifier
        .lock()
        .expect("verifier lock")
        .verify(token)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    Ok(Identity {
        id: claims.subject,
        name: claims.name,
        roles: claims
            .roles
            .iter()
            .filter_map(|role| world::Role::parse(role))
            .collect(),
    })
}

async fn scrape(State(http): State<Http>) -> String {
    metrics_process::Collector::default().collect();
    http.prometheus.render()
}

async fn health(State(http): State<Http>) -> Response {
    let ready = http.verifier.lock().expect("verifier lock").ready();
    if ready {
        "ok".into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "auth keys unavailable").into_response()
    }
}

fn unix_now() -> Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after the unix epoch")
}

fn metrics_recorder() -> metrics_exporter_prometheus::PrometheusHandle {
    metrics_process::Collector::default().describe();
    metrics_exporter_prometheus::PrometheusBuilder::new()
        .set_buckets(&[
            0.0005, 0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5,
        ])
        .expect("non-empty histogram buckets")
        .install_recorder()
        .expect("prometheus recorder installs")
}

fn verifier() -> Arc<Mutex<auth::Verifier>> {
    #[derive(serde::Deserialize)]
    struct AuthConfig {
        issuer: String,
        audience: String,
        jwks_uri: String,
    }
    let config: AuthConfig = envy::prefixed("RIFT_AUTH_")
        .from_env()
        .expect("RIFT_AUTH_* environment");
    let mut verifier = auth::Verifier::new(&config.issuer, &config.audience, &config.jwks_uri);
    match verifier.warm() {
        Ok(()) => println!("auth enabled, issuer {}", config.issuer),
        Err(error) => println!(
            "auth enabled, issuer {} (jwks warm-up failed: {error})",
            config.issuer
        ),
    }
    Arc::new(Mutex::new(verifier))
}
