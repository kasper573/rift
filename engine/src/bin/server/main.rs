mod transport;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_replication::ClientLink;
use engine::core::content::id::Leaf;
use engine::core::content::sync::{ServedContent, well_formed_hash};
use engine::core::content::{
    Content, ContentScope, FilesystemFolder, LoadPurpose, version, with_content,
};
use engine::core::math::Rng;
use engine::core::net::SessionRequest;
use engine::core::net::auth::TokenClaims;
use engine::core::net::transport::{STREAM, token_text};
use engine::core::os::StdinClosed;
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
use engine::systems::{GameSettings, TICK_HZ, load_content, server_app, start_server};
use metrics::{counter, gauge, histogram};
use renet2::{DisconnectReason, ServerEvent};
use transport::{TOKEN_LIFETIME, TokenMinter, UdpTransport};

service::heap_profiling!();

fn main() {
    if std::env::args().any(|arg| arg == BUILD_INFO_FLAG) {
        println!("{}", build_info_line());
        return;
    }
    engine::core::par::start();
    let launch = LaunchSettings::of_this_binary();
    let mut config: ServerConfig = launch.get(SERVER_ENV_PREFIX);
    config.content_dir = launch.path(&config.content_dir);
    let stop = Stop::watch(config.exit_on_stdin_eof);
    let http_bind = SocketAddr::new(config.bind, config.port);
    let udp_bind = SocketAddr::new(config.bind, config.udp_port);

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");

    let content = load_content(
        Arc::new(FilesystemFolder(config.content_dir.clone())),
        "the server did not start",
        LoadPurpose::Play,
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
    let (transport, minter) = UdpTransport::bind(udp_bind, config.public_ip, config.max_clients);
    let http = Http {
        sessions: sessions.clone(),
        minter,
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
    let daylight = config.daylight;
    let places: Vec<_> = cells.iter().collect();
    let mut worlds = Cellar {
        apps: places.iter().map(|_| None).collect(),
        cells: cells.clone(),
        make: Box::new(move |ordinal| {
            let (area, cell) = places[ordinal];
            let mut app = server_app(
                area,
                cell,
                ordinal as u64,
                rng_seed,
                wall_clock(reset),
                daylight,
                &content,
            );
            app.insert_resource(rules);
            start_server(&mut app);
            app
        }),
    };
    worlds.reach(lobby.spawn);
    // Opened only once players can connect, so a passing health check means the game is playable.
    runtime.spawn(api);

    let ready = ReadyLine {
        ready: ServerReady {
            http: http_addr.port(),
            udp: transport.first_port(),
        },
        build: BuildInfo::current(),
    };
    println!("{}", serde_json::to_string(&ready).expect("a ready line"));
    simulate(worlds, transport, sessions, lobby, stages, reset, stop);
    runtime.shutdown_background();
}

/// The world of every cell of every area, each made when a client first comes within reach of it:
/// a land of many cells costs only the worlds its players have come near.
struct Cellar {
    apps: Vec<Option<App>>,
    cells: Worlds,
    make: Box<dyn Fn(usize) -> App>,
}

impl Cellar {
    /// The world of cell `index`, made now if it wasn't.
    fn app(&mut self, index: usize) -> &mut App {
        let make = &self.make;
        self.apps[index].get_or_insert_with(|| make(index))
    }

    /// Makes every world a tick may touch on account of a client in `world`.
    fn reach(&mut self, world: usize) {
        for index in self.cells.reached_from(world) {
            self.app(index);
        }
    }

    fn count(&self) -> usize {
        self.apps.len()
    }

    fn made(&mut self) -> impl Iterator<Item = &mut App> {
        self.apps.iter_mut().flatten()
    }

    /// Every cell's world as a tick takes them, none for those not made.
    fn views(&mut self) -> Vec<Option<&mut World>> {
        self.apps
            .iter_mut()
            .map(|app| app.as_mut().map(App::world_mut))
            .collect()
    }
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
        account: AccountId,
        conns: &HashMap<u64, Conn>,
        worlds: usize,
    ) -> (ClientId, usize, Option<JoinAs>) {
        let mut accounts = self.online.0.lock().expect("online lock");
        // Another client of this account is already connected: share its player id and join in
        // whichever area that player currently occupies, so both connections drive the one
        // character (last input wins).
        if let Some(&client) = accounts.get(&account) {
            let world = conns
                .values()
                .find(|conn| conn.client == client)
                .map_or(self.spawn, |conn| conn.world);
            return (client, world, None);
        }
        let client = ClientId(self.next_client);
        self.next_client += 1;
        let join_as = self
            .test_characters
            .as_ref()
            .and_then(|characters| characters.take(&account));
        accounts.insert(account, client);
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
    mut worlds: Cellar,
    mut transport: UdpTransport,
    sessions: Sessions,
    mut lobby: Lobby,
    stages: Receiver<StageAsked>,
    reset: UtcHour,
    stop: Stop,
) {
    let mut conns: HashMap<u64, Conn> = HashMap::new();
    let frame = TICK_HZ.period();
    let mut last = Instant::now();
    loop {
        // A server told to stop says goodbye, so its players' clients know at once rather than
        // once it falls silent, and reconnect to whatever comes after it.
        if stop.asked() {
            transport.disconnect_all();
            return;
        }
        let started = Instant::now();
        let dt = started - last;
        last = started;

        for event in transport.update(dt) {
            match event {
                ServerEvent::ClientConnected { client_id } => {
                    counter!("rift_client_connections_opened_total").increment(1);
                    let Some(identity) = sessions.take(client_id) else {
                        transport.disconnect(client_id);
                        continue;
                    };
                    let (client, world, join_as) =
                        lobby.admit(identity.id.clone(), &conns, worlds.count());
                    let entity =
                        spawn_conn(worlds.app(world), client, ClientLink::default(), identity);
                    if let Some(join_as) = join_as {
                        worlds
                            .app(world)
                            .world_mut()
                            .entity_mut(entity)
                            .insert(join_as);
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
                        worlds.app(conn.world).world_mut().despawn(conn.entity);
                        if !conns.values().any(|other| other.client == conn.client) {
                            lobby.leave(conn.client);
                        }
                    }
                }
            }
        }

        for (&network_id, conn) in &conns {
            let world = worlds.app(conn.world).world_mut();
            let mut link = world
                .get_mut::<ClientLink>(conn.entity)
                .expect("a connection's link");
            while let Some(bytes) = transport.receive(network_id, STREAM) {
                link.receive(bytes);
            }
        }

        for asked in stages.try_iter() {
            let staged = stage(&mut worlds, &conns, &lobby.online, &asked);
            let _ = asked.answer.send(staged);
        }

        for conn in conns.values() {
            worlds.reach(conn.world);
        }
        let mut views = worlds.views();
        let moved = worlds::tick(&lobby.cells, &mut views, wall_clock(reset));
        for moved in moved {
            move_client(&mut worlds, &mut conns, moved.client, moved.to);
        }
        travel(&mut worlds, &mut conns, &lobby.cells);

        for (&network_id, conn) in &conns {
            let world = worlds.app(conn.world).world_mut();
            let mut link = world
                .get_mut::<ClientLink>(conn.entity)
                .expect("a connection's link");
            if link.misbehaved() {
                transport.disconnect(network_id);
            }
            for bytes in link.take_outgoing() {
                transport.send(network_id, STREAM, bytes);
            }
        }
        transport.send_packets();

        counter!("rift_ticks_total").increment(1);
        histogram!(TICK_METRIC).record(started.elapsed().as_secs_f64());
        histogram!("rift_tick_interval_seconds").record(dt.as_secs_f64());
        gauge!("rift_clients_connected").set(conns.len() as f64);
        record_ecs_metrics(&mut worlds);
        record_network_metrics(&mut transport, &conns);

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
    worlds: &mut Cellar,
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
    staging::stage(
        worlds.app(conn.world).world_mut(),
        conn.entity,
        asked.staging,
    )
}

fn wall_clock(reset: UtcHour) -> WallClock {
    WallClock {
        now: SystemTime::now().into(),
        reset,
    }
}

fn record_ecs_metrics(worlds: &mut Cellar) {
    let worlds: Vec<&mut App> = worlds.made().collect();
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

fn record_network_metrics(transport: &mut UdpTransport, conns: &HashMap<u64, Conn>) {
    let mut sent_per_sec = 0.0;
    let mut received_per_sec = 0.0;
    let mut max_packet_loss = 0.0;
    for &network_id in conns.keys() {
        if let Some(info) = transport.network_info(network_id) {
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

/// What tells the server to stop: a termination signal, or its stdin closing when it watches it.
struct Stop {
    signalled: Arc<AtomicBool>,
    stdin: Option<StdinClosed>,
}

impl Stop {
    fn watch(stdin: bool) -> Stop {
        let signalled = Arc::new(AtomicBool::new(false));
        let raise = signalled.clone();
        ctrlc::set_handler(move || raise.store(true, Ordering::Relaxed))
            .expect("install stop handler");
        Stop {
            signalled,
            stdin: stdin.then(StdinClosed::watch),
        }
    }

    fn asked(&self) -> bool {
        self.signalled.load(Ordering::Relaxed)
            || self.stdin.as_ref().is_some_and(StdinClosed::is_closed)
    }
}

struct Conn {
    world: usize,
    entity: Entity,
    client: ClientId,
}

// Logged, so whatever happened can be played out again.
fn seeded(configured: Option<u64>) -> u64 {
    let seed = configured.unwrap_or_else(Rng::random_seed);
    println!("rng seed {seed}");
    seed
}

/// The whole connection bundle is spawned in one call so `Add` observers on any of its
/// components (e.g. greeting) observe the complete connection, `Identity` included.
fn spawn_conn(app: &mut App, client: ClientId, link: ClientLink, identity: Identity) -> Entity {
    app.world_mut().spawn((link, client, identity)).id()
}

/// Every travelling player's character placed in the world of the place it is bound for, its
/// connections with it.
fn travel(worlds: &mut Cellar, conns: &mut HashMap<u64, Conn>, cells: &Worlds) {
    let travelers: Vec<_> = worlds
        .made()
        .flat_map(|app| transition::departing(app.world_mut()))
        .collect();
    for traveler in travelers {
        let dest = cells.world_of(Place {
            area: traveler.dest_area,
            at: traveler.dest,
        });
        move_client(worlds, conns, traveler.client, dest);
        transition::arrive(worlds.app(dest).world_mut(), traveler);
    }
}

/// Every connection of `client` handed over to the world `dest`, which shows them that world
/// afresh.
fn move_client(worlds: &mut Cellar, conns: &mut HashMap<u64, Conn>, client: ClientId, dest: usize) {
    worlds.reach(dest);
    for conn in conns.values_mut().filter(|conn| conn.client == client) {
        let mut leaving = worlds.app(conn.world).world_mut().entity_mut(conn.entity);
        let identity = leaving.take::<Identity>().expect("a connection's identity");
        let link = leaving.take::<ClientLink>().expect("a connection's link");
        leaving.despawn();
        conn.world = dest;
        conn.entity = spawn_conn(worlds.app(dest), client, link.hand_over(), identity);
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
        map.retain(|_, pending| now.duration_since(pending.minted) < TOKEN_LIFETIME);
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
    minter: TokenMinter,
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
    let router = service::track(router.with_state(http), metrics);
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

    let client_id = getrandom::u64().expect("os entropy for a client id");
    http.sessions.put(client_id, identity);
    (
        [
            (header::CONTENT_TYPE, "text/plain"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        token_text(&http.minter.mint(client_id)),
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
