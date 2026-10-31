mod transport;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_replicon::prelude::{ConnectedClient, RepliconChannels};
use bevy_replicon::shared::backend::server_messages::ServerMessages;
use bevy_replicon_renet2::RenetChannelsExt;
use engine::core::content::id::Leaf;
use engine::core::content::sync::{ServedContent, well_formed_hash};
use engine::core::content::{Content, ContentScope, FilesystemFolder, version, with_content};
use engine::core::math::Rng;
use engine::core::net::auth::TokenClaims;
use engine::core::net::{MAX_MESSAGE_BYTES, SessionRequest};
use engine::core::platform::StdinClosed;
use engine::core::time::{UtcHour, WallClock};
use engine::systems::account::{AccountId, Identity};
use engine::systems::area::{self, transition};
use engine::systems::launch::{
    AuthMode, BUILD_INFO_FLAG, BuildInfo, LaunchSettings, ReadyLine, SERVER_ENV_PREFIX,
    ServerConfig, ServerReady, TEST_CHARACTER_ROUTE, TICK_METRIC, TestCharacterRequest,
    build_info_line,
};
use engine::systems::player::snapshot::{self, CharacterSnapshot, JoinAs};
use engine::systems::player::{ClientId, SpawnPolicy, SpawnRules};
use engine::systems::script::Place;
use engine::systems::staging::{self, STAGING_ROUTE, Staging};
use engine::systems::worlds::{self, Worlds};
use engine::systems::{
    GameSettings, REPLICATION_INTERVAL, TICK_HZ, load_content, server_app, start_server,
};
use metrics::{counter, gauge, histogram};
use renet2::{ConnectionConfig, DisconnectReason, RenetServer, ServerEvent};
use tower_http::cors::{Any, CorsLayer};
use transport::WsTransport;

service::heap_profiling!();

const TOKEN_EXPIRE: Duration = Duration::from_secs(30);

fn main() {
    if std::env::args().any(|arg| arg == BUILD_INFO_FLAG) {
        println!("{}", build_info_line());
        return;
    }
    let launch = LaunchSettings::of_this_binary();
    let mut config: ServerConfig = launch.get(SERVER_ENV_PREFIX);
    config.content_dir = launch.path(&config.content_dir);
    let stdin = config.exit_on_stdin_eof.then(StdinClosed::watch);
    let http_bind = SocketAddr::new(config.bind, config.port);
    let ws_bind = SocketAddr::new(config.bind, config.ws_port);

    // One multi-threaded tokio runtime backs both the HTTP API and the WebSocket transport's
    // accept/read/write tasks, for as long as the simulation runs.
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");

    let content = load_content(
        Arc::new(FilesystemFolder(config.content_dir.clone())),
        "the server did not start",
    )
    .unwrap_or_else(|report| {
        eprint!("{report}");
        std::process::exit(2);
    });
    let settings = content.built::<GameSettings>();
    let served = ServedContent::new(
        &content,
        &FilesystemFolder(config.content_dir.clone()),
        version::build(),
        &settings.name,
    )
    .unwrap_or_else(|error| {
        eprintln!("{}: {error}", config.content_dir.display());
        std::process::exit(2);
    });

    let metrics = service::recorder();
    let sessions = Sessions::default();
    let online = Online::default();
    let (stage_sender, stages) = channel();
    let http = Http {
        sessions: sessions.clone(),
        auth: match config.auth {
            AuthMode::Oidc => Auth::Oidc(verifier(launch.get(oidc::ENV_PREFIX))),
            AuthMode::Fake => Auth::Fake,
        },
        test_characters: config.test_characters.then(|| TestCharacters {
            content: content.clone(),
            pending: Arc::default(),
            online: online.clone(),
            stages: Arc::new(Mutex::new(stage_sender)),
        }),
        served: Arc::new(served),
        game: service::GameInfo {
            name: settings.name.clone(),
        },
    };
    let listener = runtime
        .block_on(tokio::net::TcpListener::bind(http_bind))
        .unwrap_or_else(|error| panic!("cannot bind {http_bind}: {error}"));
    let http_addr = listener.local_addr().expect("bound http address");
    let cells = Worlds::of(&content);
    let lobby = Lobby {
        spawn: cells.world_of(settings.initial_spawn),
        cells: cells.clone(),
        policy: config.player_spawn_policy,
        online,
        test_characters: http.test_characters.clone(),
        next_client: 1,
        next_dealt: 0,
    };
    let api = serve_http(listener, http, metrics);

    let rules = SpawnRules {
        policy: config.player_spawn_policy,
        immortal: config.player_immortal,
    };
    let reset = config.daily_reset_utc_hour;
    let rng_seed = seeded(config.rng_seed);
    let worlds: Vec<App> = cells
        .iter()
        .enumerate()
        .map(|(ordinal, (area, cell))| {
            let mut app = server_app(
                area,
                cell,
                ordinal as u64,
                rng_seed,
                wall_clock(reset),
                config.daylight,
                &content,
            );
            app.insert_resource(rules);
            start_server(&mut app);
            app
        })
        .collect();
    let transport = WsTransport::bind(
        ws_bind,
        sessions,
        config.max_clients,
        runtime.handle().clone(),
    );
    // Opened only once players can connect, so a passing health check means the game is playable.
    runtime.spawn(api);

    let ready = ReadyLine {
        ready: ServerReady {
            http: http_addr.port(),
            ws: transport.local_addr().port(),
        },
        build: BuildInfo::current(),
    };
    println!("{}", serde_json::to_string(&ready).expect("a ready line"));
    simulate(worlds, transport, lobby, stages, reset, stdin);
    runtime.shutdown_background();
}

struct Lobby {
    /// The world of the game's initial spawn.
    spawn: usize,
    cells: Worlds,
    policy: SpawnPolicy,
    online: Online,
    test_characters: Option<TestCharacters>,
    next_client: u32,
    next_dealt: usize,
}

impl Lobby {
    fn admit(
        &mut self,
        account: Option<AccountId>,
        conns: &HashMap<u64, Conn>,
        worlds: usize,
    ) -> (ClientId, usize, Option<JoinAs>) {
        let mut accounts = self.online.0.lock().expect("online lock");
        // Another tab of this account is already connected: share its player id and join in
        // whichever area that player currently occupies, so both sockets drive the one character
        // (last input wins).
        if let Some(&client) = account.as_ref().and_then(|account| accounts.get(account)) {
            let world = conns
                .values()
                .find(|conn| conn.client == client)
                .map_or(self.spawn, |conn| conn.world);
            return (client, world, None);
        }
        let client = ClientId(self.next_client);
        self.next_client += 1;
        let join_as = account.as_ref().and_then(|account| {
            self.test_characters
                .as_ref()
                .and_then(|characters| characters.take(account))
        });
        if let Some(account) = account {
            accounts.insert(account, client);
        }
        let world = match (&join_as, self.policy) {
            (Some(join_as), _) => self.cells.world_of(join_as.start),
            (None, SpawnPolicy::Dist) => {
                let world = self.next_dealt % worlds;
                self.next_dealt += 1;
                world
            }
            (None, SpawnPolicy::Map) => self.spawn,
        };
        (client, world, join_as)
    }

    fn leave(&self, client: ClientId) {
        self.online
            .0
            .lock()
            .expect("online lock")
            .retain(|_, &mut online| online != client);
    }
}

fn simulate(
    mut worlds: Vec<App>,
    mut transport: WsTransport,
    mut lobby: Lobby,
    stages: Receiver<StageAsked>,
    reset: UtcHour,
    stdin: Option<StdinClosed>,
) {
    let (connection_config, client_channels) = {
        let channels = worlds[0].world().resource::<RepliconChannels>();
        let config =
            ConnectionConfig::from_channels(channels.server_configs(), channels.client_configs());
        (config, channels.client_channels().len())
    };
    let mut server = RenetServer::new(connection_config);
    let mut conns: HashMap<u64, Conn> = HashMap::new();
    let mut transfers: Vec<Transfer> = Vec::new();
    let mut tick = 0u64;
    let frame = TICK_HZ.period();
    let mut last = Instant::now();
    loop {
        if stdin.as_ref().is_some_and(StdinClosed::is_closed) {
            return;
        }
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
                    let (client, world, join_as) = lobby.admit(account, &conns, worlds.len());
                    let entity = spawn_conn(&mut worlds[world], client, client_id, identity);
                    if let Some(join_as) = join_as {
                        worlds[world].world_mut().entity_mut(entity).insert(join_as);
                    }
                    conns.insert(
                        client_id,
                        Conn {
                            world,
                            entity,
                            client,
                        },
                    );
                }
                ServerEvent::ClientDisconnected { client_id, reason } => {
                    counter!("rift_client_connections_closed_total", "code" => disconnect_code(&reason))
                        .increment(1);
                    if let Some(conn) = conns.remove(&client_id) {
                        worlds[conn.world].world_mut().despawn(conn.entity);
                        if !conns.values().any(|other| other.client == conn.client) {
                            lobby.leave(conn.client);
                        }
                    }
                }
            }
        }

        for (&network_id, conn) in &conns {
            let world = worlds[conn.world].world_mut();
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

        for asked in stages.try_iter() {
            let staged = stage(&mut worlds, &conns, &lobby.online, &asked);
            let _ = asked.answer.send(staged);
        }

        let mut views: Vec<&mut World> = worlds.iter_mut().map(App::world_mut).collect();
        for moved in worlds::tick(&lobby.cells, &mut views, wall_clock(reset)) {
            transfers.push(Transfer {
                client: moved.client,
                dest: moved.to,
                departed_tick: tick,
                traveler: None,
            });
        }

        begin_transfers(&mut worlds, &lobby.cells, &mut transfers, tick);

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
        histogram!(TICK_METRIC).record(started.elapsed().as_secs_f64());
        histogram!("rift_tick_interval_seconds").record(dt.as_secs_f64());
        gauge!("rift_clients_connected").set(conns.len() as f64);
        record_ecs_metrics(&worlds);
        record_network_metrics(&server, &conns);

        if let Some(remaining) = frame.checked_sub(started.elapsed()) {
            std::thread::sleep(remaining);
        }
    }
}

/// A scene asked for the character of `account`, and where to tell how it went.
struct StageAsked {
    account: AccountId,
    staging: Staging,
    answer: tokio::sync::oneshot::Sender<Result<(), String>>,
}

fn stage(
    worlds: &mut [App],
    conns: &HashMap<u64, Conn>,
    online: &Online,
    asked: &StageAsked,
) -> Result<(), String> {
    let client = online
        .0
        .lock()
        .expect("online lock")
        .get(&asked.account)
        .copied()
        .ok_or("the account is in no world")?;
    let conn = conns
        .values()
        .find(|conn| conn.client == client)
        .ok_or("the account's character is between areas")?;
    staging::stage(worlds[conn.world].world_mut(), conn.entity, asked.staging)
}

fn wall_clock(reset: UtcHour) -> WallClock {
    WallClock {
        now: SystemTime::now().into(),
        reset,
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
    world: usize,
    entity: Entity,
    client: ClientId,
}

#[derive(Component)]
struct Wire(u64);

// Logged, so whatever happened can be played out again.
fn seeded(configured: Option<u64>) -> u64 {
    let seed = configured.unwrap_or_else(Rng::random_seed);
    println!("rng seed {seed}");
    seed
}

/// The whole connection bundle is spawned in one call so `Add` observers on any of its
/// components (e.g. greeting) observe the complete connection, `Identity` included.
fn spawn_conn(
    app: &mut App,
    client: ClientId,
    network_id: u64,
    identity: Option<Identity>,
) -> Entity {
    let world = app.world_mut();
    let base = (
        ConnectedClient {
            max_size: MAX_MESSAGE_BYTES,
        },
        client,
        Wire(network_id),
    );
    match identity {
        Some(identity) => world.spawn((base, identity)).id(),
        None => world.spawn(base).id(),
    }
}

/// A client (and all its connections) that left its world and is waiting to be re-created in the
/// destination world: a player whose character warped to another area. Leaving blinds the
/// connections to the source world, and the wait lets the resulting despawns reach them first, so
/// their replication state is empty before the destination's fresh snapshot arrives over the same
/// connections — no entity-id or tick collision between the two worlds. The wait is one
/// replication period (not one tick): a world only replicates every `REPLICATION_INTERVAL` ticks,
/// so the despawns aren't guaranteed sent until then.
struct Transfer {
    client: ClientId,
    dest: usize,
    departed_tick: u64,
    /// The character to place on arrival; none when it already stands there, handed over by
    /// its cell's world, and only the connection follows.
    traveler: Option<Box<transition::Traveler>>,
}

/// Phase 1: take everyone leaving their world this tick out of it, and queue the move.
fn begin_transfers(worlds: &mut [App], cells: &Worlds, transfers: &mut Vec<Transfer>, tick: u64) {
    for app in worlds.iter_mut() {
        for traveler in transition::departing(app.world_mut()) {
            transfers.push(Transfer {
                client: traveler.client,
                dest: cells.world_of(Place {
                    area: traveler.dest_area,
                    at: traveler.dest,
                }),
                departed_tick: tick,
                traveler: Some(Box::new(traveler)),
            });
        }
    }
}

/// Phase 2: once the source world's despawns have been sent (a replication period after departing) —
/// move every socket of the client to the destination world and re-create the player there.
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
            client,
            dest,
            traveler,
            ..
        } = transfers.remove(index);
        let movers: Vec<(u64, usize, Entity)> = conns
            .iter()
            .filter(|(_, conn)| conn.client == client)
            .map(|(&network_id, conn)| (network_id, conn.world, conn.entity))
            .collect();
        if movers.is_empty() {
            continue;
        }
        for (network_id, world, entity) in movers {
            let identity = worlds[world].world().get::<Identity>(entity).cloned();
            worlds[world].world_mut().despawn(entity);
            let entity = spawn_conn(&mut worlds[dest], client, network_id, identity);
            conns.insert(
                network_id,
                Conn {
                    world: dest,
                    entity,
                    client,
                },
            );
        }
        if let Some(traveler) = traveler {
            transition::arrive(worlds[dest].world_mut(), *traveler);
        }
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

#[derive(Clone, Default)]
struct Online(Arc<Mutex<HashMap<AccountId, ClientId>>>);

#[derive(Clone)]
struct TestCharacters {
    content: Content,
    pending: Arc<Mutex<HashMap<AccountId, JoinAs>>>,
    online: Online,
    stages: Arc<Mutex<Sender<StageAsked>>>,
}

impl TestCharacters {
    fn take(&self, account: &AccountId) -> Option<JoinAs> {
        self.pending.lock().expect("pending lock").remove(account)
    }
}

#[derive(Clone)]
enum Auth {
    Oidc(Arc<Mutex<oidc::TokenVerifier>>),
    Fake,
}

#[derive(Clone)]
struct Http {
    sessions: Sessions,
    auth: Auth,
    test_characters: Option<TestCharacters>,
    served: Arc<ServedContent>,
    game: service::GameInfo,
}

async fn serve_http(
    listener: tokio::net::TcpListener,
    http: Http,
    metrics: service::PrometheusHandle,
) {
    let mut router = axum::Router::new()
        .route("/session", post(session))
        .route("/health", get(health))
        .route(service::GAME_INFO_ROUTE, get(game))
        .route("/content/manifest", get(manifest))
        .route("/content/files/{hash}", get(content_file));
    if http.test_characters.is_some() {
        router = router
            .route(TEST_CHARACTER_ROUTE, post(test_character))
            .route(STAGING_ROUTE, post(stage_scene));
    }
    // A browser client plays from the game's site, another origin. Nothing here reads a cookie (a
    // bearer token signs a session in, and the content is public), so any site may call it.
    let browsers = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION]);
    let router = service::track(router.with_state(http).layer(browsers), metrics);
    ctrlc::set_handler(|| std::process::exit(0)).expect("install stop handler");
    axum::serve(listener, router).await.expect("axum serves");
}

async fn game(State(http): State<Http>) -> Response {
    (
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        serde_json::to_string(&http.game).expect("the game's info serializes"),
    )
        .into_response()
}

async fn manifest(State(http): State<Http>) -> Response {
    (
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        serde_json::to_string(&http.served.manifest).expect("manifest serializes"),
    )
        .into_response()
}

async fn content_file(
    State(http): State<Http>,
    axum::extract::Path(hash): axum::extract::Path<String>,
) -> Response {
    if !well_formed_hash(&hash) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    match http.served.file(&hash) {
        Some(bytes) => (
            [
                (header::CONTENT_TYPE, "application/octet-stream"),
                (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
            ],
            bytes.to_vec(),
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn session(State(http): State<Http>, headers: HeaderMap, body: String) -> Response {
    let authorization = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let identity = match resolve(&http, authorization) {
        Ok(identity) => identity,
        Err(status) => return status.into_response(),
    };
    let request: SessionRequest = match serde_json::from_str(&body) {
        Ok(request) => request,
        Err(error) => return (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    };
    if request.build != version::build() {
        return (StatusCode::CONFLICT, "the server runs another build").into_response();
    }
    if request.rows != http.served.manifest.rows.hash {
        return (StatusCode::CONFLICT, "the server serves other content").into_response();
    }

    // The ticket is the renet client id. The browser opens `wss://host/?ticket=<id>`; the WebSocket
    // handshake claims it single-use. Unguessable in 2^64, ~30s expiry, only ever sent over TLS.
    let client_id = getrandom::u64().expect("os entropy for a session ticket");
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

async fn test_character(State(http): State<Http>, headers: HeaderMap, body: String) -> Response {
    let Some(characters) = http.test_characters.as_ref() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let authorization = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let identity = match resolve(&http, authorization) {
        Ok(identity) => identity,
        Err(status) => return status.into_response(),
    };
    let content = &characters.content;
    let (request, scope) = with_content(ContentScope::of(content), || {
        serde_json::from_str::<TestCharacterRequest>(&body)
    });
    let request = match request {
        Ok(request) => request,
        Err(error) => return (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    };
    if let Some(problem) = scope.leaves().iter().find_map(Leaf::reported) {
        return (StatusCode::BAD_REQUEST, problem.to_string()).into_response();
    }
    let Some(character) = content.by_path::<CharacterSnapshot>(&request.test_character) else {
        return (
            StatusCode::BAD_REQUEST,
            format!("no test character `{}`", request.test_character),
        )
            .into_response();
    };
    let start = request
        .start
        .unwrap_or_else(|| snapshot::start_of(content, character.get(content)));
    if let Err(problem) = area::check_place(content, start) {
        return (StatusCode::BAD_REQUEST, format!("bad start: {problem}")).into_response();
    }
    if characters
        .online
        .0
        .lock()
        .expect("online lock")
        .contains_key(&identity.id)
    {
        return (StatusCode::CONFLICT, "already in a world").into_response();
    }
    characters.pending.lock().expect("pending lock").insert(
        identity.id,
        JoinAs {
            snapshot: character.get(content).clone(),
            start,
        },
    );
    StatusCode::NO_CONTENT.into_response()
}

async fn stage_scene(State(http): State<Http>, headers: HeaderMap, body: String) -> Response {
    let Some(characters) = http.test_characters.as_ref() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let authorization = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let identity = match resolve(&http, authorization) {
        Ok(identity) => identity,
        Err(status) => return status.into_response(),
    };
    let (staging, scope) = with_content(ContentScope::of(&characters.content), || {
        serde_json::from_str::<Staging>(&body)
    });
    let staging = match staging {
        Ok(staging) => staging,
        Err(error) => return (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    };
    if let Some(problem) = scope.leaves().iter().find_map(Leaf::reported) {
        return (StatusCode::BAD_REQUEST, problem.to_string()).into_response();
    }
    let (answer, staged) = tokio::sync::oneshot::channel();
    let asked = StageAsked {
        account: identity.id,
        staging,
        answer,
    };
    if characters
        .stages
        .lock()
        .expect("stages lock")
        .send(asked)
        .is_err()
    {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    match staged.await {
        Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
        Ok(Err(problem)) => (StatusCode::CONFLICT, problem).into_response(),
        Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

fn resolve(http: &Http, authorization: &str) -> Result<Identity, StatusCode> {
    let token = authorization
        .strip_prefix("Bearer ")
        .unwrap_or(authorization);
    let claims = match &http.auth {
        Auth::Oidc(verifier) => verifier
            .lock()
            .expect("verifier lock")
            .verify::<TokenClaims>(token)
            .ok(),
        Auth::Fake => TokenClaims::read_unverified(token),
    }
    .filter(|claims| !claims.sub.is_empty())
    .ok_or(StatusCode::UNAUTHORIZED)?;
    Ok(Identity {
        name: claims.display_name(),
        id: AccountId(claims.sub),
    })
}

async fn health(State(http): State<Http>) -> Response {
    let ready = match &http.auth {
        Auth::Oidc(verifier) => verifier.lock().expect("verifier lock").ready(),
        Auth::Fake => true,
    };
    if ready {
        "ok".into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "auth keys unavailable").into_response()
    }
}

fn verifier(settings: oidc::TokenCheckSettings) -> Arc<Mutex<oidc::TokenVerifier>> {
    let mut verifier = oidc::TokenVerifier::new(settings);
    match verifier.warm() {
        Ok(()) => println!("auth enabled, issuer {}", verifier.issuer()),
        Err(error) => println!(
            "auth enabled, issuer {} (jwks warm-up failed: {error})",
            verifier.issuer()
        ),
    }
    Arc::new(Mutex::new(verifier))
}
