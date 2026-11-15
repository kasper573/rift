mod transport;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_replication::ClientLink;
use engine::core::content::id::Leaf;
use engine::core::content::sync::{FILES, MANIFEST, ServedContent, well_formed_hash};
use engine::core::content::{
    Content, ContentScope, FilesystemFolder, LoadPurpose, version, with_content,
};
use engine::core::math::Rng;
use engine::core::net::SessionRequest;
use engine::core::net::auth::TokenClaims;
use engine::core::net::transport::{STREAM, token_text};
use engine::core::os::StdinClosed;
use engine::core::store::{Record, RecordId, RecordStore, RecordStoreProblem};
use engine::core::time::{UnixMillis, UtcHour, WallClock};
use engine::systems::account::{AccountId, AccountIdentity};
use engine::systems::area::{self, transition};
use engine::systems::launch::{
    AuthMode, BUILD_INFO_FLAG, BuildInfo, LaunchSettings, ReadyLine, SERVER_ENV_PREFIX,
    ServerConfig, ServerDaylight, ServerReady, TEST_CHARACTER_ROUTE, TICK_METRIC,
    TestCharacterRequest, build_info_line,
};
use engine::systems::player::snapshot::{self, CharacterSnapshot, JoinAs, JoinOrigin};
use engine::systems::player::{ClientId, Players, SpawnPolicy, SpawnRules};
use engine::systems::save::load::{self, LoadedCharacter};
use engine::systems::save::{
    self, Checkpoints, DepartedCharacters, GameChange, Returned, SavedCharacter,
};
use engine::systems::society::{ServerSociety, Society};
use engine::systems::staging::{self, STAGING_ROUTE, Staging};
use engine::systems::worlds::{self, AboveWorlds, Worlds};
use engine::systems::{GameSettings, TICK_HZ, load_content, server_app, start_server};
use metrics::{counter, gauge, histogram};
use renet2::{DisconnectReason, ServerEvent};
use serde::de::DeserializeOwned;
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
    let store = runtime
        .block_on(RecordStore::open(&config.store))
        .unwrap_or_else(|problem| {
            eprintln!(
                "the server did not start: nothing can be saved to {}: {problem}",
                config.store
            );
            std::process::exit(2);
        });

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
        engine::systems::registry(),
        &content,
        &FilesystemFolder(config.content_dir.clone()),
        version::build(),
        &settings.name,
    )
    .unwrap_or_else(|error| {
        eprintln!("{}: {error}", config.content_dir.display());
        std::process::exit(2);
    });

    let society = runtime
        .block_on(Society::read(
            &store,
            &ContentScope::of(&content),
            engine::systems::request_sources(),
        ))
        .unwrap_or_else(|problem| {
            eprintln!(
                "the server did not start: what {} holds cannot be read: {problem}",
                config.store
            );
            std::process::exit(2);
        });
    let above = AboveWorlds {
        roster: Default::default(),
        society: ServerSociety::of(society),
        checkpoints: Checkpoints::to(
            store.clone(),
            runtime.handle().clone(),
            &content,
            config.checkpoint_every,
            TOKEN_LIFETIME + READ_PATIENCE,
        ),
    };

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
            stages: stage_sender,
        }),
        served: Arc::new(served),
        content: content.clone(),
        store,
        online: online.clone(),
        departed: above.checkpoints.memory(),
        game: service::GameInfo {
            name: settings.name.clone(),
        },
    };
    let listener = runtime
        .block_on(tokio::net::TcpListener::bind(http_bind))
        .unwrap_or_else(|error| panic!("cannot bind {http_bind}: {error}"));
    let http_addr = listener.local_addr().expect("bound http address");
    let areas = Worlds::of(&content);
    let lobby = Lobby {
        spawn: areas.world_of(settings.initial_spawn.area),
        areas: areas.clone(),
        content: content.clone(),
        policy: config.player_spawn_policy,
        online,
        places: HashMap::new(),
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
    let day = config.day();
    assert!(
        day.daylight != ServerDaylight::Fixed || day.hour.is_some(),
        "{SERVER_ENV_PREFIX}HOUR: a fixed daylight names the hour it holds at"
    );
    let weather = config.weather.course(wall_clock(reset).now);
    let running: Vec<_> = areas.iter().collect();
    let mut worlds = AreaWorlds {
        apps: running.iter().map(|_| None).collect(),
        make: Box::new(move |ordinal| {
            let mut app = server_app(
                running[ordinal],
                ordinal as u64,
                rng_seed,
                wall_clock(reset),
                day,
                weather,
                &content,
            );
            app.insert_resource(rules);
            start_server(&mut app);
            app
        }),
    };
    worlds.app(lobby.spawn);
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
    simulate(
        worlds, above, transport, sessions, lobby, stages, reset, stop,
    );
    runtime.shutdown_background();
}

/// The world of every area, each made when a client first comes to it: a game of many areas
/// costs only the worlds its players have been to.
struct AreaWorlds {
    apps: Vec<Option<App>>,
    make: Box<dyn Fn(usize) -> App>,
}

impl AreaWorlds {
    /// The world of area `index`, made now if it wasn't.
    fn app(&mut self, index: usize) -> &mut App {
        let make = &self.make;
        self.apps[index].get_or_insert_with(|| make(index))
    }

    fn count(&self) -> usize {
        self.apps.len()
    }

    fn made(&mut self) -> impl Iterator<Item = &mut App> {
        self.apps.iter_mut().flatten()
    }

    /// Every area's world as a tick takes them, none for those not made.
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
    areas: Worlds,
    content: Content,
    policy: SpawnPolicy,
    online: Online,
    /// The world each character stands in, its player there or gone.
    places: HashMap<ClientId, usize>,
    test_characters: Option<TestCharacters>,
    next_client: u32,
    next_dealt: usize,
}

/// A connection let in: the client it plays as, the world it joins, the character it joins as
/// unless it makes or shares one, and what the game changing did to that character.
struct Admitted {
    client: ClientId,
    world: usize,
    join_as: Option<JoinAs>,
    changes: Vec<GameChange>,
}

impl Lobby {
    /// Lets in a connection of `identity`. A character it comes back to is the one memory holds,
    /// newer than anything the store has of it, and only then the one its sign-in read from the
    /// store. Refused when its sign-in read nothing because the account had a character in play
    /// that is in play no longer: a fresh character never stands in for one that exists.
    fn admit(
        &mut self,
        identity: &AccountIdentity,
        found: SessionCharacter,
        above: &mut AboveWorlds,
        worlds: usize,
        now: UnixMillis,
    ) -> Option<Admitted> {
        let mut accounts = self.online.0.lock().expect("online lock");
        // The account's character is in a world already, its player there or gone: this
        // connection shares its player id and joins it where it stands, so every connection of
        // the account drives the one character (last input wins).
        if let Some(&client) = accounts.get(&identity.id) {
            return Some(Admitted {
                client,
                world: self.places.get(&client).copied().unwrap_or(self.spawn),
                join_as: None,
                changes: Vec::new(),
            });
        }
        let staged = self
            .test_characters
            .as_ref()
            .and_then(|characters| characters.take(&identity.id));
        let departed = above.checkpoints.departed(&identity.id);
        let (join_as, changes) = match (staged, departed, found) {
            (Some(join_as), ..) => (Some(join_as), Vec::new()),
            (None, Some(saved), _) => saved_character(load::unchanged(&self.content, &saved)),
            (None, None, SessionCharacter::Read(Some(loaded))) => saved_character(*loaded),
            (None, None, SessionCharacter::Read(None)) => (None, Vec::new()),
            (None, None, SessionCharacter::InPlay) => return None,
        };
        let client = ClientId(self.next_client);
        self.next_client += 1;
        accounts.insert(identity.id.clone(), client);
        above
            .society
            .signed_in(identity, now, &mut above.checkpoints.unwritten);
        let world = match (&join_as, self.policy) {
            (Some(join_as), _) => self.areas.world_of(join_as.start.area),
            (None, SpawnPolicy::Dist) => {
                let world = self.next_dealt % worlds;
                self.next_dealt += 1;
                world
            }
            (None, SpawnPolicy::Map) => self.spawn,
        };
        self.places.insert(client, world);
        Some(Admitted {
            client,
            world,
            join_as,
            changes,
        })
    }

    /// The character `client` played left its world.
    fn leave(&mut self, client: ClientId) {
        self.places.remove(&client);
        self.online
            .0
            .lock()
            .expect("online lock")
            .retain(|_, &mut online| online != client);
    }
}

#[expect(clippy::too_many_arguments)]
fn simulate(
    mut worlds: AreaWorlds,
    mut above: AboveWorlds,
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
        // A server told to stop saves the game as it stands and then says goodbye, so its
        // players' clients know at once rather than once it falls silent, and reconnect to
        // whatever comes after it, where they left.
        if stop.asked() {
            save_before_stopping(&mut worlds, &mut above, reset, &stop);
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
                    let Some(session) = sessions.take(client_id) else {
                        transport.disconnect(client_id);
                        continue;
                    };
                    let Some(admitted) = lobby.admit(
                        &session.identity,
                        session.found,
                        &mut above,
                        worlds.count(),
                        wall_clock(reset).now,
                    ) else {
                        transport.disconnect(client_id);
                        continue;
                    };
                    let world = admitted.world;
                    let entity = spawn_conn(
                        worlds.app(world),
                        admitted.client,
                        ClientLink::default(),
                        session.identity,
                    );
                    let mut conn = worlds.app(world).world_mut().entity_mut(entity);
                    if let Some(join_as) = admitted.join_as {
                        conn.insert(join_as);
                    }
                    if !admitted.changes.is_empty() {
                        conn.insert(Returned {
                            changes: admitted.changes,
                        });
                    }
                    conns.insert(
                        client_id,
                        Conn {
                            world,
                            entity,
                            client: admitted.client,
                        },
                    );
                }
                ServerEvent::ClientDisconnected { client_id, reason } => {
                    counter!("rift_client_connections_closed_total", "code" => disconnect_code(&reason))
                        .increment(1);
                    // The character it played stays in its world a while yet, and its world
                    // says when it leaves. A connection that closes before it has a character
                    // leaves nothing behind.
                    if let Some(conn) = conns.remove(&client_id) {
                        let world = worlds.app(conn.world).world_mut();
                        world.despawn(conn.entity);
                        let plays = world.resource::<Players>().0.contains_key(&conn.client);
                        if !plays && !conns.values().any(|other| other.client == conn.client) {
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

        let mut views = worlds.views();
        for departure in worlds::tick(&mut views, wall_clock(reset), &mut above) {
            lobby.leave(departure.client);
        }
        travel(&mut worlds, &mut conns, &mut lobby);

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
        record_saving_metrics(&mut above.checkpoints);

        if let Some(remaining) = frame.checked_sub(started.elapsed()) {
            std::thread::sleep(remaining);
        }
    }
}

/// Saves the game as it stands and waits until it is written, trying again for as long as the
/// store refuses it: whoever runs the server decides how long that may take, and a second
/// signal to stop gives up at once.
fn save_before_stopping(
    worlds: &mut AreaWorlds,
    above: &mut AboveWorlds,
    reset: UtcHour,
    stop: &Stop,
) {
    loop {
        let mut views = worlds.views();
        let saved = save::save_all(&mut views, &mut above.checkpoints, wall_clock(reset).now);
        record_saving_metrics(&mut above.checkpoints);
        match saved {
            Ok(()) => return,
            Err(problem) if stop.insisted() => {
                eprintln!("the server stops with the game not saved: {problem}");
                return;
            }
            Err(problem) => {
                eprintln!("the game is not saved yet, trying again: {problem}");
                std::thread::sleep(SAVE_AGAIN_AFTER);
            }
        }
    }
}

/// How long a server that is stopping waits before it tries again to save a game its store
/// refused.
const SAVE_AGAIN_AFTER: Duration = Duration::from_secs(2);

/// How long a sign-in waits for the store to hand over what it holds of the account, before it
/// is refused.
const READ_PATIENCE: Duration = Duration::from_secs(10);

fn record_saving_metrics(checkpoints: &mut Checkpoints) {
    for report in checkpoints.reports() {
        match &report.outcome {
            Ok(written) => {
                counter!("rift_checkpoints_total", "outcome" => "written").increment(1);
                counter!("rift_checkpoint_records_written_total").increment(*written as u64);
            }
            Err(problem) => {
                counter!("rift_checkpoints_total", "outcome" => "failed").increment(1);
                eprintln!("a checkpoint was not written: {problem}");
            }
        }
        histogram!("rift_checkpoint_duration_seconds").record(report.took.as_secs_f64());
    }
    gauge!("rift_records_unwritten").set(checkpoints.waiting() as f64);
    if let Some(saved) = checkpoints.last_saved() {
        gauge!("rift_last_checkpoint_timestamp_seconds").set(saved.0 as f64 / 1000.0);
    }
}

/// A scene asked for the character of `account`, and where to tell how it went.
struct StageAsked {
    account: AccountId,
    staging: Staging,
    answer: tokio::sync::oneshot::Sender<Result<(), String>>,
}

fn stage(
    worlds: &mut AreaWorlds,
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
        asked.staging.clone(),
    )
}

fn wall_clock(reset: UtcHour) -> WallClock {
    WallClock {
        now: SystemTime::now().into(),
        reset,
    }
}

fn record_ecs_metrics(worlds: &mut AreaWorlds) {
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
    signals: Arc<AtomicU32>,
    stdin: Option<StdinClosed>,
}

impl Stop {
    fn watch(stdin: bool) -> Stop {
        let signals = Arc::new(AtomicU32::new(0));
        let raise = signals.clone();
        ctrlc::set_handler(move || {
            raise.fetch_add(1, Ordering::Relaxed);
        })
        .expect("install stop handler");
        Stop {
            signals,
            stdin: stdin.then(StdinClosed::watch),
        }
    }

    fn asked(&self) -> bool {
        self.signals.load(Ordering::Relaxed) > 0
            || self.stdin.as_ref().is_some_and(StdinClosed::is_closed)
    }

    /// Whether it was signalled to stop again since it first was.
    fn insisted(&self) -> bool {
        self.signals.load(Ordering::Relaxed) > 1
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
/// components observe the complete connection, `Identity` included.
fn spawn_conn(
    app: &mut App,
    client: ClientId,
    link: ClientLink,
    identity: AccountIdentity,
) -> Entity {
    app.world_mut().spawn((link, client, identity)).id()
}

/// Every travelling player's character placed in the world of the place it is bound for, its
/// connections with it.
fn travel(worlds: &mut AreaWorlds, conns: &mut HashMap<u64, Conn>, lobby: &mut Lobby) {
    let travelers: Vec<_> = worlds
        .made()
        .flat_map(|app| transition::departing(app.world_mut()))
        .collect();
    for traveler in travelers {
        let dest = lobby.areas.world_of(traveler.dest_area);
        let client = traveler.played.client;
        move_client(worlds, conns, client, dest);
        lobby.places.insert(client, dest);
        transition::arrive(worlds.app(dest).world_mut(), traveler);
    }
}

/// Every connection of `client` handed over to the world `dest`, which shows them that world
/// afresh.
fn move_client(
    worlds: &mut AreaWorlds,
    conns: &mut HashMap<u64, Conn>,
    client: ClientId,
    dest: usize,
) {
    for conn in conns.values_mut().filter(|conn| conn.client == client) {
        let mut leaving = worlds.app(conn.world).world_mut().entity_mut(conn.entity);
        let identity = leaving
            .take::<AccountIdentity>()
            .expect("a connection's identity");
        let link = leaving.take::<ClientLink>().expect("a connection's link");
        leaving.despawn();
        conn.world = dest;
        conn.entity = spawn_conn(worlds.app(dest), client, link.hand_over(), identity);
    }
}

#[derive(Resource, Clone, Default)]
struct Sessions(Arc<Mutex<HashMap<u64, Pending>>>);

/// A sign-in that was handed its connect token: who it is, what it was found to play when it
/// asked, and when the token was minted.
struct Pending {
    identity: AccountIdentity,
    found: SessionCharacter,
    minted: Instant,
}

impl Sessions {
    fn put(&self, client_id: u64, identity: AccountIdentity, found: SessionCharacter) {
        let mut map = self.0.lock().expect("sessions lock");
        let now = Instant::now();
        map.retain(|_, pending| now.duration_since(pending.minted) < TOKEN_LIFETIME);
        map.insert(
            client_id,
            Pending {
                identity,
                found,
                minted: now,
            },
        );
    }

    fn take(&self, client_id: u64) -> Option<Pending> {
        self.0.lock().expect("sessions lock").remove(&client_id)
    }
}

/// What a sign-in was found to play as it was handed its connect token.
enum SessionCharacter {
    /// A character already in play, in a world or staged for it: nothing was read.
    InPlay,
    /// What is saved of the account's own character: nothing, for one that never played.
    Read(Option<Box<LoadedCharacter>>),
}

/// The saved character a connection joins as, and what the game changing did to it.
fn saved_character(loaded: LoadedCharacter) -> (Option<JoinAs>, Vec<GameChange>) {
    let join_as = JoinAs {
        snapshot: loaded.saved.character,
        start: loaded.start,
        origin: JoinOrigin::Saved {
            at: loaded.saved.saved_at,
            told: loaded.saved.told,
        },
    };
    (Some(join_as), loaded.changes)
}

/// The accounts whose character is in a world, its player there or gone, and the client that
/// plays each.
#[derive(Clone, Default)]
struct Online(Arc<Mutex<HashMap<AccountId, ClientId>>>);

impl Online {
    fn plays(&self, account: &AccountId) -> bool {
        self.0.lock().expect("online lock").contains_key(account)
    }
}

#[derive(Clone)]
struct TestCharacters {
    content: Content,
    pending: Arc<Mutex<HashMap<AccountId, JoinAs>>>,
    online: Online,
    stages: Sender<StageAsked>,
}

impl TestCharacters {
    fn take(&self, account: &AccountId) -> Option<JoinAs> {
        self.pending.lock().expect("pending lock").remove(account)
    }

    fn waits_for(&self, account: &AccountId) -> bool {
        self.pending
            .lock()
            .expect("pending lock")
            .contains_key(account)
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
    content: Content,
    store: RecordStore,
    online: Online,
    departed: DepartedCharacters,
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
        .route(MANIFEST, get(manifest))
        .route(&format!("{FILES}{{hash}}"), get(content_file));
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
    let identity = match resolve(&http, &headers) {
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

    let found = match find_character(&http, &identity.id).await {
        Ok(found) => found,
        Err(problem) => {
            eprintln!("the sign-in of {} is refused: {problem}", identity.id.0);
            return (StatusCode::SERVICE_UNAVAILABLE, RECORDS_UNREACHABLE).into_response();
        }
    };

    let client_id = getrandom::u64().expect("os entropy for a client id");
    http.sessions.put(client_id, identity, found);
    (
        [
            (header::CONTENT_TYPE, "text/plain"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        token_text(&http.minter.mint(client_id)),
    )
        .into_response()
}

/// The character of `account` as it is saved, as the game it is read under takes it: the one
/// memory holds since it left a world, and only without one what the store holds. An account
/// about to play a test character plays no character of its own, and one whose character is in
/// a world plays that, so neither is read. Refused when the store does not answer in time or
/// holds a character this build cannot read: a saved character that could not be read is never
/// stood in for by a fresh one.
async fn find_character(
    http: &Http,
    account: &AccountId,
) -> Result<SessionCharacter, RecordStoreProblem> {
    let staged = http
        .test_characters
        .as_ref()
        .is_some_and(|characters| characters.waits_for(account));
    if staged || http.online.plays(account) {
        return Ok(SessionCharacter::InPlay);
    }
    if let Some(saved) = http.departed.of(account) {
        let loaded = load::unchanged(&http.content, &saved);
        return Ok(SessionCharacter::Read(Some(Box::new(loaded))));
    }
    let id = RecordId(account.0.clone());
    let held = tokio::time::timeout(READ_PATIENCE, http.store.one(SavedCharacter::KIND, &id))
        .await
        .map_err(|_| RecordStoreProblem("the store took too long to answer".to_owned()))??;
    let loaded = held.map(|body| load::load(&http.content, &body).map(Box::new));
    loaded.transpose().map(SessionCharacter::Read)
}

/// What a sign-in is told when what is saved of its account cannot be read.
const RECORDS_UNREACHABLE: &str = "the game cannot reach its records";

async fn test_character(State(http): State<Http>, headers: HeaderMap, body: String) -> Response {
    let Some(characters) = http.test_characters.as_ref() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let identity = match resolve(&http, &headers) {
        Ok(identity) => identity,
        Err(status) => return status.into_response(),
    };
    let content = &characters.content;
    let request = match parse_in::<TestCharacterRequest>(content, &body) {
        Ok(request) => request,
        Err(rejection) => return rejection.into_response(),
    };
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
            origin: JoinOrigin::Staged,
        },
    );
    StatusCode::NO_CONTENT.into_response()
}

async fn stage_scene(State(http): State<Http>, headers: HeaderMap, body: String) -> Response {
    let Some(characters) = http.test_characters.as_ref() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let identity = match resolve(&http, &headers) {
        Ok(identity) => identity,
        Err(status) => return status.into_response(),
    };
    let staging = match parse_in::<Staging>(&characters.content, &body) {
        Ok(staging) => staging,
        Err(rejection) => return rejection.into_response(),
    };
    let (answer, staged) = tokio::sync::oneshot::channel();
    let asked = StageAsked {
        account: identity.id,
        staging,
        answer,
    };
    if characters.stages.send(asked).is_err() {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    match staged.await {
        Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
        Ok(Err(problem)) => (StatusCode::CONFLICT, problem).into_response(),
        Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

fn parse_in<T: DeserializeOwned>(content: &Content, body: &str) -> Result<T, (StatusCode, String)> {
    let (parsed, scope) = with_content(ContentScope::of(content), || {
        serde_json::from_str::<T>(body)
    });
    let parsed = parsed.map_err(|error| (StatusCode::BAD_REQUEST, error.to_string()))?;
    match scope.leaves().iter().find_map(Leaf::reported) {
        Some(problem) => Err((StatusCode::BAD_REQUEST, problem.to_string())),
        None => Ok(parsed),
    }
}

fn resolve(http: &Http, headers: &HeaderMap) -> Result<AccountIdentity, StatusCode> {
    let authorization = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
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
    Ok(AccountIdentity::of(&http.content, claims))
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
