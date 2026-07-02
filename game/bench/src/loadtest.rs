//! Load generator: connects headless replicon clients to an already-running real server over real
//! websocket sockets and holds them idle. Start the server with `just loadtest-stack-up` first.
//!
//! Each client drives renet2's core directly over a plain WebSocket — the same netcode-free transport
//! the browser and the server use — so there is no client cap. `dyn` mode root-finds the player count
//! whose server tick hits the budget, using the same projection as the area benchmark.

mod search;

use std::io::ErrorKind;
use std::net::TcpStream;
use std::time::{Duration, Instant};

use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_ecs::schedule::IntoScheduleConfigs;
use bevy_replicon::prelude::*;
use bevy_state::prelude::*;
use renet2::{ConnectionConfig, RenetClient};
use tungstenite::Message;
use tungstenite::stream::MaybeTlsStream;
use world::core::channels::RenetChannelsExt;
use world::systems::player::session::{self, ClientSessionPlugin};

const TICK: Duration = Duration::from_millis(33);
const BUDGET_MS: f64 = 40.0;
/// Search clamp for the player root-find; the real limit is the server's client cap or its tick time.
const MAX_PLAYERS: usize = 50_000;
/// Open this many sockets before pumping, so already-connected clients keep draining while we connect.
const CONNECT_BATCH: usize = 100;
/// Let the server settle at a new player count before sampling its tick.
const STABILIZE: Duration = Duration::from_secs(2);
/// Sample the server tick over this window.
const WINDOW: Duration = Duration::from_secs(3);

#[derive(serde::Deserialize)]
struct Config {
    http_url: String,
    ws_url: String,
}

fn main() {
    let config: Config = envy::prefixed("RIFT_LOADTEST_")
        .from_env()
        .expect("RIFT_LOADTEST_* environment");
    let mode = std::env::args().nth(1);

    match mode.as_deref() {
        None | Some("dyn") => run_dyn(&config.http_url, &config.ws_url),
        Some(spec) => match spec.parse::<usize>() {
            Ok(count) => run_fixed(&config.http_url, &config.ws_url, count),
            Err(_) => {
                eprintln!("usage: loadtest [dyn | <count>]");
                std::process::exit(1);
            }
        },
    }
}

fn run_fixed(http_url: &str, ws_url: &str, count: usize) {
    println!("[loadtest] connecting {count} clients to {ws_url} (fake tokens via {http_url})");
    let started = Instant::now();
    let mut clients: Vec<App> = Vec::new();
    let mut next_id = 0usize;
    set_player_count(&mut clients, http_url, ws_url, &mut next_id, count);
    println!(
        "[loadtest] opened {}/{count} in {:.1}s — holding (Ctrl-C to stop)",
        connected_count(&clients),
        started.elapsed().as_secs_f64()
    );
    let mut report = Instant::now();
    loop {
        pump(&mut clients, TICK);
        if report.elapsed() >= Duration::from_secs(2) {
            println!(
                "[loadtest] connected {}/{}",
                connected_count(&clients),
                clients.len()
            );
            report = Instant::now();
        }
    }
}

/// Root-find the highest player count the server sustains within the tick budget, using the same
/// projection the area benchmark uses. Each probe adjusts the live client pool to the target count
/// (connect or disconnect) rather than restarting, so the search converges in a handful of probes.
fn run_dyn(http_url: &str, ws_url: &str) {
    println!("[loadtest] root-finding the {BUDGET_MS:.0}ms/tick player equilibrium ({ws_url})");
    let mut clients: Vec<App> = Vec::new();
    let mut next_id = 0usize;
    let mut under: Option<(usize, f64)> = None;
    let mut over: Option<(usize, f64)> = None;
    let mut previous: Option<(usize, f64)> = None;
    let mut next = Some(1usize);
    while let Some(target) = next {
        set_player_count(&mut clients, http_url, ws_url, &mut next_id, target);
        let live = connected_count(&clients);
        let ms = measure_tick_ms(http_url, &mut clients, WINDOW);
        let verdict = if ms <= BUDGET_MS { "ok" } else { "over" };
        println!("[loadtest]   {live:>6} players  →  server {ms:6.2}ms/tick  {verdict}");

        let last = (live, ms);
        if ms <= BUDGET_MS {
            if under.is_none_or(|(highest, _)| live >= highest) {
                under = Some(last);
            }
        } else if over.is_none_or(|(lowest, _)| live <= lowest) {
            over = Some(last);
        }
        // Couldn't OPEN as many sockets as asked (not merely "not all connected yet") while still under
        // budget: the limit is the machine's or server's connection ceiling, not tick time, and that
        // count is the answer. Compare opened sockets, not `live`, so a slow handshake isn't mistaken
        // for a ceiling.
        if clients.len() < target && ms < BUDGET_MS {
            println!("[loadtest] connection ceiling at {live} players (could not open more)");
            break;
        }
        next = search::project(BUDGET_MS, MAX_PLAYERS, under, over, previous, last);
        previous = Some(last);
    }
    match under {
        Some((players, ms)) => println!(
            "[loadtest] equilibrium: ~{players} concurrent players sustained at {ms:.1}ms/tick (budget {BUDGET_MS:.0}ms)"
        ),
        None => println!("[loadtest] the first probe already exceeded the budget"),
    }
}

/// Brings the live client pool to exactly `target` players: drops surplus clients (each dropped
/// websocket closes, which the server sees as a disconnect) or opens new ones. New clients connect in
/// batches, pumping the pool between batches so already-connected sockets keep draining; a batch that
/// opens nothing means a connection ceiling was hit. Fake ids are monotonic so a reconnecting slot is
/// never mistaken for a still-connected account.
fn set_player_count(
    clients: &mut Vec<App>,
    http_url: &str,
    ws_url: &str,
    next_id: &mut usize,
    target: usize,
) {
    clients.truncate(target);
    while clients.len() < target {
        let want = CONNECT_BATCH.min(target - clients.len());
        let before = clients.len();
        for _ in 0..want {
            let id = *next_id;
            *next_id += 1;
            if let Some(app) = build_client(http_url, ws_url, id) {
                clients.push(app);
            }
        }
        if clients.len() == before {
            break;
        }
        pump(clients, Duration::from_millis(100));
    }
    pump(clients, STABILIZE);
}

fn pump(clients: &mut [App], duration: Duration) {
    let deadline = Instant::now() + duration;
    loop {
        let frame = Instant::now();
        for client in clients.iter_mut() {
            client.update();
        }
        if let Some(remaining) = TICK.checked_sub(frame.elapsed()) {
            std::thread::sleep(remaining);
        }
        if Instant::now() >= deadline {
            return;
        }
    }
}

fn measure_tick_ms(http_url: &str, clients: &mut [App], window: Duration) -> f64 {
    let before = scrape_tick(http_url);
    pump(clients, window);
    let after = scrape_tick(http_url);
    match (before, after) {
        (Some((sum0, count0)), Some((sum1, count1))) if count1 > count0 => {
            (sum1 - sum0) / (count1 - count0) * 1000.0
        }
        _ => 0.0,
    }
}

fn scrape_tick(http_url: &str) -> Option<(f64, f64)> {
    let body = ureq::get(format!("{http_url}/metrics"))
        .call()
        .ok()?
        .body_mut()
        .read_to_string()
        .ok()?;
    let mut sum = None;
    let mut count = None;
    for line in body.lines() {
        let value = || line.rsplit(' ').next().and_then(|v| v.parse::<f64>().ok());
        if line.starts_with("rift_tick_duration_seconds_sum") {
            sum = value();
        } else if line.starts_with("rift_tick_duration_seconds_count") {
            count = value();
        }
    }
    Some((sum?, count?))
}

fn connected_count(clients: &[App]) -> usize {
    clients
        .iter()
        .filter(|app| {
            matches!(
                app.world().resource::<State<ClientState>>().get(),
                ClientState::Connected
            )
        })
        .count()
}

fn build_client(http_url: &str, ws_url: &str, index: usize) -> Option<App> {
    let ticket = fetch_ticket(http_url, &format!("loadtest-{index}"))?;
    let socket = WsSocket::connect(ws_url, &ticket)?;

    let mut app = App::new();
    app.add_plugins((bevy_time::TimePlugin, bevy_state::app::StatesPlugin));
    app.add_plugins(ClientSessionPlugin);
    app.add_plugins(BridgePlugin);
    app.init_resource::<Joined>();
    app.add_systems(bevy_app::Update, announce_join);

    let config = {
        let channels = app.world().resource::<RepliconChannels>();
        ConnectionConfig::from_channels(channels.server_configs(), channels.client_configs())
    };
    // The websocket is reliable+ordered (TCP), matching the server side.
    app.insert_resource(Client(RenetClient::new(config, true)));
    app.insert_non_send(Socket(socket));
    app.finish();
    app.cleanup();
    Some(app)
}

fn fetch_ticket(http_url: &str, fake_id: &str) -> Option<String> {
    let mut response = ureq::post(format!("{http_url}/session"))
        .header("Authorization", format!("fake:{fake_id}"))
        .send_empty()
        .ok()?;
    Some(response.body_mut().read_to_string().ok()?.trim().to_owned())
}

/// A blocking-then-nonblocking WebSocket carrying opaque renet packets as binary frames.
struct WsSocket {
    ws: tungstenite::WebSocket<MaybeTlsStream<TcpStream>>,
    open: bool,
}

impl WsSocket {
    fn connect(ws_url: &str, ticket: &str) -> Option<WsSocket> {
        let (ws, _response) = tungstenite::connect(format!("{ws_url}/?ticket={ticket}"))
            .map_err(|error| eprintln!("[ws] connect: {error}"))
            .ok()?;
        if let MaybeTlsStream::Plain(tcp) = ws.get_ref() {
            tcp.set_nonblocking(true).ok()?;
        }
        Some(WsSocket { ws, open: true })
    }

    fn recv(&mut self) -> Option<Vec<u8>> {
        match self.ws.read() {
            Ok(Message::Binary(data)) => Some(data.to_vec()),
            Ok(Message::Close(_)) => {
                self.open = false;
                None
            }
            Ok(_) => None,
            Err(tungstenite::Error::Io(error)) if error.kind() == ErrorKind::WouldBlock => None,
            Err(_) => {
                self.open = false;
                None
            }
        }
    }

    fn send(&mut self, packet: Vec<u8>) {
        match self.ws.send(Message::Binary(packet.into())) {
            Ok(()) => {}
            // A non-blocking write that couldn't flush fully is buffered for the next flush.
            Err(tungstenite::Error::Io(error)) if error.kind() == ErrorKind::WouldBlock => {}
            Err(_) => self.open = false,
        }
    }

    fn flush(&mut self) {
        let _ = self.ws.flush();
    }
}

// Mirrors the client's replicon<->renet2 bridge (game/client/src/core/net/transport.rs).
#[derive(Resource)]
struct Client(RenetClient);

struct Socket(WsSocket);

struct BridgePlugin;

impl bevy_app::Plugin for BridgePlugin {
    fn build(&self, app: &mut App) {
        use bevy_app::{PostUpdate, PreUpdate};
        app.add_systems(
            PreUpdate,
            (
                drive,
                set_connecting.run_if(in_state(ClientState::Disconnected).and_then(connecting)),
                promote.run_if(in_state(ClientState::Connecting).and_then(connecting)),
                set_connected.run_if(in_state(ClientState::Connecting).and_then(connected)),
                set_disconnected.run_if(connection_lost),
                receive_packets.run_if(connected),
            )
                .chain()
                .in_set(ClientSystems::ReceivePackets),
        )
        .add_systems(
            PostUpdate,
            send_packets
                .run_if(connected)
                .in_set(ClientSystems::SendPackets),
        );
    }
}

fn drive(mut socket: NonSendMut<Socket>, mut client: ResMut<Client>, time: Res<bevy_time::Time>) {
    client.0.update(time.delta());
    while let Some(packet) = socket.0.recv() {
        client.0.process_packet(&packet);
    }
    // renet has no internal timeout, so a closed socket is the only liveness signal — otherwise a
    // server-reaped client would stay "connected" and inflate the measured player count.
    if !socket.0.open {
        client.0.disconnect_due_to_transport();
    }
}

// renet2's core opens in `Connecting` and stays there until told otherwise; with no netcode layer the
// transport promotes it once the socket is up and replicon has reached `Connecting`, so the state
// machine advances Disconnected -> Connecting -> Connected in order.
fn promote(socket: NonSend<Socket>, mut client: ResMut<Client>) {
    if socket.0.open {
        client.0.set_connected();
    }
}

// Once the server's welcome arrives (`MyClient` set), announce the join exactly as the real client
// does — without it the socket connects but no player character is ever spawned server-side, so the
// simulation runs with zero real players.
#[derive(Resource, Default)]
struct Joined(bool);

fn announce_join(world: &mut World) {
    if world.resource::<Joined>().0 || session::my_id(world).is_none() {
        return;
    }
    session::join(world);
    world.resource_mut::<Joined>().0 = true;
}

fn set_connecting(mut state: ResMut<NextState<ClientState>>) {
    state.set(ClientState::Connecting);
}

fn set_connected(mut state: ResMut<NextState<ClientState>>) {
    state.set(ClientState::Connected);
}

fn set_disconnected(mut state: ResMut<NextState<ClientState>>) {
    state.set(ClientState::Disconnected);
}

fn receive_packets(
    channels: Res<RepliconChannels>,
    mut client: ResMut<Client>,
    mut messages: ResMut<ClientMessages>,
) {
    for channel_id in 0..channels.server_channels().len() as u8 {
        while let Some(message) = client.0.receive_message(channel_id) {
            messages.insert_received(channel_id, message);
        }
    }
}

fn send_packets(
    mut socket: NonSendMut<Socket>,
    mut client: ResMut<Client>,
    mut messages: ResMut<ClientMessages>,
) {
    for (channel_id, message) in messages.drain_sent() {
        client.0.send_message(channel_id as u8, message);
    }
    for packet in client.0.get_packets_to_send() {
        socket.0.send(packet);
    }
    socket.0.flush();
}

fn connecting(client: Option<Res<Client>>) -> bool {
    client.is_some_and(|client| client.0.is_connecting())
}

fn connected(client: Option<Res<Client>>) -> bool {
    client.is_some_and(|client| client.0.is_connected())
}

fn connection_lost(state: Res<State<ClientState>>, client: Option<Res<Client>>) -> bool {
    !matches!(state.get(), ClientState::Disconnected)
        && client.is_some_and(|client| client.0.is_disconnected())
}
