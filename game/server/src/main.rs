mod auth;
mod transport;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_replicon::prelude::{ConnectedClient, RepliconChannels, ServerState};
use bevy_replicon::shared::backend::server_messages::ServerMessages;
use bevy_state::prelude::NextState;
use metrics::{counter, gauge, histogram};
use rand::RngCore;
use renet2::{ConnectionConfig, DisconnectReason, RenetServer, ServerEvent};
use strum::VariantArray;
use transport::WsTransport;
use world::core::assets::{AssetService, FilesystemSource};
use world::core::channels::RenetChannelsExt;
use world::systems::account::Identity;
use world::systems::area::transition;
use world::systems::player::{ClientId, Immortal, SpawnPolicy};
use world::systems::{REPLICATION_INTERVAL, TICK_HZ};

service::heap_profiling!();

const TOKEN_EXPIRE: Duration = Duration::from_secs(30);

#[derive(serde::Deserialize)]
struct Config {
    port: u16,
    ws_port: u16,
    max_clients: usize,
    areas: Areas,
    assets_dir: PathBuf,
    player_spawn_type: SpawnPolicy,
    player_immortal: bool,
    allow_fake_users: bool,
    pyroscope_enabled: bool,
    pyroscope_sample_hz: u32,
}

#[derive(Clone, Copy)]
enum Areas {
    Real,
    Bench(usize),
}

impl<'de> serde::Deserialize<'de> for Areas {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Areas, D::Error> {
        let spec = String::deserialize(deserializer)?;
        if spec == "real" {
            return Ok(Areas::Real);
        }
        if let Some(count) = spec.strip_prefix("bench,") {
            return count
                .parse()
                .map(Areas::Bench)
                .map_err(serde::de::Error::custom);
        }
        Err(serde::de::Error::custom(format!(
            "expected `real` or `bench,<n>`, got `{spec}`"
        )))
    }
}

fn main() {
    let config: Config = envy::prefixed("RIFT_GAME_SERVER_")
        .from_env()
        .expect("RIFT_GAME_SERVER_* environment");
    let _profiler = service::profiler(
        "rift-game-server",
        config.pyroscope_enabled,
        config.pyroscope_sample_hz,
    );
    let http_bind: SocketAddr = format!("0.0.0.0:{}", config.port)
        .parse()
        .expect("server bind address");
    let ws_bind: SocketAddr = format!("0.0.0.0:{}", config.ws_port)
        .parse()
        .expect("websocket bind address");

    // One multi-threaded tokio runtime backs both the HTTP API and the WebSocket transport's
    // accept/read/write tasks; it outlives `main` because `simulate` never returns.
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");

    let metrics = service::recorder();
    let sessions = Sessions::default();
    let http = Http {
        sessions: sessions.clone(),
        verifier: verifier(),
        allow_fake_users: config.allow_fake_users,
    };
    runtime.spawn(serve_http(http_bind, http, metrics));

    let settings = Settings {
        areas: config.areas,
        max_clients: config.max_clients,
        spawn_policy: config.player_spawn_type,
        immortal: Immortal(config.player_immortal),
    };
    let assets = AssetService::new(FilesystemSource(config.assets_dir));
    simulate(
        ws_bind,
        sessions,
        runtime.handle().clone(),
        assets,
        settings,
    );
}

struct Settings {
    areas: Areas,
    max_clients: usize,
    spawn_policy: SpawnPolicy,
    immortal: Immortal,
}

fn simulate(
    ws_bind: SocketAddr,
    sessions: Sessions,
    runtime: tokio::runtime::Handle,
    assets: AssetService,
    settings: Settings,
) {
    let Settings {
        areas,
        max_clients,
        spawn_policy,
        immortal,
    } = settings;
    let area_ids = select_areas(areas);
    let spawn = area_ids
        .iter()
        .position(|id| *id == world::data::area::SPAWN_ID)
        .unwrap_or(0);
    let mut worlds: Vec<App> = area_ids
        .iter()
        .enumerate()
        .map(|(ordinal, &id)| build_world(id, &assets, spawn_policy, immortal, ordinal as u64))
        .collect();

    let (connection_config, client_channels) = {
        let channels = worlds[0].world().resource::<RepliconChannels>();
        let config =
            ConnectionConfig::from_channels(channels.server_configs(), channels.client_configs());
        (config, channels.client_channels().len())
    };
    let mut server = RenetServer::new(connection_config);
    let mut transport = WsTransport::bind(ws_bind, sessions, max_clients, runtime);

    println!(
        "mmo server listening: websocket {ws_bind} ({} area worlds)",
        worlds.len()
    );

    let mut conns: HashMap<u64, Conn> = HashMap::new();
    let mut accounts: HashMap<String, ClientId> = HashMap::new();
    let mut transfers: Vec<Transfer> = Vec::new();
    let mut next_client = 1u32;
    let mut next_area = 0usize;
    let mut tick = 0u64;
    let frame = TICK_HZ.period();
    let mut last = Instant::now();
    loop {
        let started = Instant::now();
        let dt = started - last;
        last = started;
        tick += 1;

        server.update(dt);
        transport.update(&mut server);

        while let Some(event) = server.get_event() {
            match event {
                ServerEvent::ClientConnected { client_id } => {
                    counter!("rift_client_connections_opened_total").increment(1);
                    let identity = transport.take_identity(client_id);
                    let account = identity.as_ref().map(|identity| identity.id.clone());
                    let (client, area) =
                        match account.as_ref().and_then(|account| accounts.get(account)) {
                            // Another tab of this account is already connected: share its player id and
                            // join in whichever area that player currently occupies, so both sockets
                            // drive the one character (last input wins).
                            Some(&client) => {
                                let area = conns
                                    .values()
                                    .find(|conn| conn.client == client)
                                    .map_or(spawn, |conn| conn.area);
                                (client, area)
                            }
                            None => {
                                let client = ClientId(next_client);
                                next_client += 1;
                                if let Some(account) = account {
                                    accounts.insert(account, client);
                                }
                                let area = match spawn_policy {
                                    SpawnPolicy::Dist => {
                                        let area = next_area % worlds.len();
                                        next_area += 1;
                                        area
                                    }
                                    SpawnPolicy::Map => spawn,
                                };
                                (client, area)
                            }
                        };
                    let entity = spawn_conn(&mut worlds[area], client, client_id, identity);
                    conns.insert(
                        client_id,
                        Conn {
                            area,
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
                        if !conns.values().any(|other| other.client == conn.client) {
                            accounts.retain(|_, &mut client| client != conn.client);
                        }
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

        world::systems::step_areas(&mut worlds);

        begin_transfers(&mut worlds, &mut transfers, tick);

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

        transport.send(&mut server);

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
        if let Ok(info) = server.network_info(network_id) {
            sent_per_sec += info.bytes_sent_per_second;
            received_per_sec += info.bytes_received_per_second;
            max_packet_loss = f64::max(max_packet_loss, info.packet_loss);
            histogram!("rift_client_rtt_seconds").record(info.rtt);
        }
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

fn select_areas(areas: Areas) -> Vec<world::data::area::Id> {
    use world::data::area::Id;
    match areas {
        Areas::Real => Id::VARIANTS.to_vec(),
        // The bench area is one template instanced `count` times; the count is bounded only by the
        // machine, not by how many area rows the content table happens to define.
        Areas::Bench(count) => vec![world::data::area::BENCH_ID; count],
    }
}

fn build_world(
    area: world::systems::area::Id,
    assets: &AssetService,
    spawn_policy: SpawnPolicy,
    immortal: Immortal,
    ordinal: u64,
) -> App {
    let mut app = world::systems::server_app(area, ordinal);
    app.insert_resource(assets.clone());
    app.insert_resource(world::core::math::Rng::from_entropy());
    app.insert_resource(spawn_policy);
    app.insert_resource(immortal);
    app.finish();
    app.cleanup();
    app.world_mut()
        .resource_mut::<NextState<ServerState>>()
        .set(ServerState::Running);
    app.update();
    app
}

/// The whole connection bundle is spawned in one call so `Add` observers on any of its
/// components (e.g. greeting, terminal tab issuance) observe the complete connection,
/// `Identity` included.
fn spawn_conn(
    app: &mut App,
    client: ClientId,
    network_id: u64,
    identity: Option<Identity>,
) -> Entity {
    let world = app.world_mut();
    let base = (ConnectedClient { max_size: 1200 }, client, Wire(network_id));
    match identity {
        Some(identity) => world.spawn((base, identity)).id(),
        None => world.spawn(base).id(),
    }
}

/// A player (and all its connections) whose character left its world (despawned) and is waiting to be
/// re-created in the destination world. The wait lets the source world's despawns reach the
/// connections first, so their replication state is empty before the destination's fresh snapshot
/// arrives over the same connections — no entity-id or tick collision between the two worlds. The
/// wait is one replication period (not one tick): a world only replicates every
/// `REPLICATION_INTERVAL` ticks, so the despawn isn't guaranteed sent until then.
struct Transfer {
    client: ClientId,
    traveler: transition::Traveler,
    departed_tick: u64,
}

/// Phase 1: despawn the character of everyone who stepped through a cross-area portal this tick (so
/// replicon despawns this world's entities for their connections), and queue the move.
fn begin_transfers(worlds: &mut [App], transfers: &mut Vec<Transfer>, tick: u64) {
    for app in worlds.iter_mut() {
        for traveler in transition::departing(app.world_mut()) {
            transfers.push(Transfer {
                client: traveler.client,
                traveler,
                departed_tick: tick,
            });
        }
    }
}

/// Phase 2: once the source world's despawns have been sent (a replication period after departing) —
/// move every socket of the moving player to the destination world and re-create the character there.
fn finish_transfers(
    worlds: &mut [App],
    conns: &mut HashMap<u64, Conn>,
    transfers: &mut Vec<Transfer>,
    tick: u64,
) {
    let mut index = 0;
    while index < transfers.len() {
        if tick < transfers[index].departed_tick + REPLICATION_INTERVAL {
            index += 1;
            continue;
        }
        let Transfer {
            client, traveler, ..
        } = transfers.remove(index);
        let dest = traveler.dest_area.index();
        let movers: Vec<(u64, usize, Entity)> = conns
            .iter()
            .filter(|(_, conn)| conn.client == client)
            .map(|(&network_id, conn)| (network_id, conn.area, conn.entity))
            .collect();
        if movers.is_empty() {
            continue;
        }
        for (network_id, area, entity) in movers {
            let identity = worlds[area].world().get::<Identity>(entity).cloned();
            worlds[area].world_mut().despawn(entity);
            let entity = spawn_conn(&mut worlds[dest], client, network_id, identity);
            conns.insert(
                network_id,
                Conn {
                    area: dest,
                    entity,
                    client,
                },
            );
        }
        transition::arrive(worlds[dest].world_mut(), traveler);
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
    allow_fake_users: bool,
}

async fn serve_http(addr: SocketAddr, http: Http, metrics: service::PrometheusHandle) {
    let router = service::track(
        axum::Router::new()
            .route("/session", post(session))
            .route("/health", get(health))
            .with_state(http),
        metrics,
    );
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

    // The ticket is the renet client id. The browser opens `wss://host/?ticket=<id>`; the WebSocket
    // handshake claims it single-use. Unguessable in 2^64, ~30s expiry, only ever sent over TLS.
    let client_id = rand::rng().next_u64();
    http.sessions.put(client_id, identity);
    (
        [
            (header::CONTENT_TYPE, "text/plain"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        client_id.to_string(),
    )
        .into_response()
}

fn resolve(http: &Http, authorization: &str) -> Result<Identity, StatusCode> {
    if http.allow_fake_users
        && let Some(id) = authorization.strip_prefix("fake:")
    {
        return Ok(Identity {
            id: id.to_owned(),
            name: id.to_owned(),
            roles: Vec::new(),
        });
    }
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
            .filter_map(|role| role.parse::<world::systems::account::Role>().ok())
            .collect(),
    })
}

async fn health(State(http): State<Http>) -> Response {
    let ready = http.verifier.lock().expect("verifier lock").ready();
    if ready {
        "ok".into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "auth keys unavailable").into_response()
    }
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
