//! Load generator: connects headless replicon clients to an already-running real server over real
//! websocket sockets and holds them idle. Start the server with `just loadtest-stack-up` first.

use std::io::{self, ErrorKind};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_ecs::schedule::IntoScheduleConfigs;
use bevy_replicon::prelude::*;
use bevy_state::prelude::*;
use renet2::{ConnectionConfig, RenetClient};
use renet2_netcode::{
    ClientAuthentication, ClientSocket, ConnectToken, NetcodeClientTransport, NetcodeTransportError,
};
use tungstenite::Message;
use tungstenite::stream::MaybeTlsStream;
use world::core::channels::RenetChannelsExt;
use world::systems::player::session::{self, ClientSessionPlugin};

// The placeholder address the server bakes into every connect token (it routes by websocket, not IP);
// `send`/`try_recv` are keyed on it.
const SERVER_ADDR: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0);

const TICK: Duration = Duration::from_millis(33);
const BUDGET_MS: f64 = 40.0;
const RAMP_STEP: usize = 50;
const NETCODE_MAX_CLIENTS: usize = 1024;

fn unix_now() -> Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
}

fn main() {
    let mode = std::env::args().nth(1);
    let http_url =
        std::env::var("RIFT_LOADTEST_HTTP_URL").unwrap_or_else(|_| "http://127.0.0.1:9998".into());
    let ws_url =
        std::env::var("RIFT_LOADTEST_WS_URL").unwrap_or_else(|_| "ws://127.0.0.1:9999".into());

    match mode.as_deref() {
        None | Some("dyn") => run_dyn(&http_url, &ws_url),
        Some(spec) => match spec.parse::<usize>() {
            Ok(count) => run_fixed(&http_url, &ws_url, count),
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
    let mut clients = connect(http_url, ws_url, 0, count);
    println!(
        "[loadtest] opened {}/{count} in {:.1}s — holding (Ctrl-C to stop)",
        clients.len(),
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

fn run_dyn(http_url: &str, ws_url: &str) {
    println!("[loadtest] ramping players to find the {BUDGET_MS:.0}ms/tick equilibrium ({ws_url})");
    let mut clients: Vec<App> = Vec::new();
    loop {
        let start = clients.len();
        clients.extend(connect(http_url, ws_url, start, RAMP_STEP));
        pump(&mut clients, Duration::from_secs(2));
        let frame_ms = measure_tick_ms(http_url, &mut clients, Duration::from_secs(3));
        let live = connected_count(&clients);
        println!("[loadtest]   {live:>5} players  →  server {frame_ms:6.2}ms/tick");

        if frame_ms >= BUDGET_MS {
            println!(
                "[loadtest] equilibrium: ~{live} concurrent players at {frame_ms:.1}ms/tick (budget {BUDGET_MS:.0}ms)"
            );
            return;
        }
        if live + RAMP_STEP > NETCODE_MAX_CLIENTS {
            println!(
                "[loadtest] reached the netcode ceiling (~{NETCODE_MAX_CLIENTS}) at only {frame_ms:.1}ms/tick — \
                 the 1024-client transport cap is the limit, not server tick time"
            );
            return;
        }
    }
}

fn connect(http_url: &str, ws_url: &str, start: usize, count: usize) -> Vec<App> {
    (start..start + count)
        .filter_map(|i| build_client(http_url, ws_url, i))
        .collect()
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
    let token_bytes = fetch_token(http_url, &format!("loadtest-{index}"))?;
    let connect_token = ConnectToken::read(&mut io::Cursor::new(token_bytes)).ok()?;

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
    let socket = WsSocket::new(ws_url);
    let client = RenetClient::new(config, socket.is_reliable());
    let transport = NetcodeClientTransport::new(
        unix_now(),
        ClientAuthentication::Secure { connect_token },
        socket,
    )
    .ok()?;
    app.insert_resource(Client(client));
    app.insert_resource(Transport(transport));
    app.finish();
    app.cleanup();
    Some(app)
}

fn fetch_token(http_url: &str, fake_id: &str) -> Option<Vec<u8>> {
    let mut response = ureq::post(format!("{http_url}/session"))
        .header("Authorization", format!("fake:{fake_id}"))
        .send_empty()
        .ok()?;
    response.body_mut().read_to_vec().ok()
}

// renet2's websocket netcode protocol carries the netcode connection request as a `creq=` URL query
// parameter on the handshake, then exchanges every later packet as a binary frame — so the socket
// can't open until the first packet (the connection request) is sent.
#[derive(Debug)]
struct WsSocket {
    url: String,
    ws: Option<tungstenite::WebSocket<MaybeTlsStream<TcpStream>>>,
    requested: bool,
    closed: bool,
}

impl WsSocket {
    fn new(url: &str) -> WsSocket {
        WsSocket {
            url: url.to_owned(),
            ws: None,
            requested: false,
            closed: false,
        }
    }

    // The connection request goes in the `creq` query parameter percent-encoded. The request is built
    // by hand (rather than `tungstenite::connect`) so the `url` crate doesn't decode that encoding back
    // into raw bytes in the request line — which the server's HTTP parser would reject.
    fn open(&mut self, connection_request: &[u8]) -> Result<(), ()> {
        let encoded = urlencoding::encode_binary(connection_request);
        let host = self
            .url
            .strip_prefix("ws://")
            .unwrap_or(&self.url)
            .to_owned();
        let stream = TcpStream::connect(&host).map_err(|error| eprintln!("[ws] tcp: {error}"))?;
        let request = tungstenite::http::Request::builder()
            .uri(format!("ws://{host}/?creq={encoded}"))
            .header("Host", &host)
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header(
                "Sec-WebSocket-Key",
                tungstenite::handshake::client::generate_key(),
            )
            .body(())
            .map_err(|error| eprintln!("[ws] request: {error}"))?;
        let (ws, _response) = tungstenite::client(request, MaybeTlsStream::Plain(stream))
            .map_err(|error| eprintln!("[ws] handshake: {error}"))?;
        if let MaybeTlsStream::Plain(tcp) = ws.get_ref() {
            let _ = tcp.set_nonblocking(true);
        }
        self.ws = Some(ws);
        Ok(())
    }
}

impl ClientSocket for WsSocket {
    // Match the server's websocket socket: it reports TLS-proxied (encrypted) and reliable.
    fn is_encrypted(&self) -> bool {
        true
    }
    fn is_reliable(&self) -> bool {
        true
    }

    fn addr(&self) -> io::Result<SocketAddr> {
        Err(io::Error::from(ErrorKind::AddrNotAvailable))
    }

    fn is_closed(&mut self) -> bool {
        self.closed
    }

    fn close(&mut self) {
        if let Some(ws) = self.ws.as_mut() {
            let _ = ws.close(None);
        }
        self.closed = true;
    }

    fn preupdate(&mut self) {}

    fn try_recv(&mut self, buffer: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        let Some(ws) = self.ws.as_mut() else {
            return Err(ErrorKind::WouldBlock.into());
        };
        match ws.read() {
            Ok(Message::Binary(data)) => {
                if data.len() > buffer.len() {
                    return Err(ErrorKind::InvalidData.into());
                }
                buffer[..data.len()].copy_from_slice(&data);
                Ok((data.len(), SERVER_ADDR))
            }
            Ok(Message::Close(_)) => {
                self.closed = true;
                Err(ErrorKind::ConnectionAborted.into())
            }
            Ok(_) => Err(ErrorKind::WouldBlock.into()),
            Err(tungstenite::Error::Io(error)) if error.kind() == ErrorKind::WouldBlock => {
                Err(ErrorKind::WouldBlock.into())
            }
            Err(_) => {
                self.closed = true;
                Err(ErrorKind::WouldBlock.into())
            }
        }
    }

    fn postupdate(&mut self) {
        if let Some(ws) = self.ws.as_mut() {
            let _ = ws.flush();
        }
    }

    fn send(&mut self, addr: SocketAddr, packet: &[u8]) -> Result<(), NetcodeTransportError> {
        if addr != SERVER_ADDR {
            return Err(io::Error::from(ErrorKind::AddrNotAvailable).into());
        }
        if !self.requested {
            self.requested = true;
            if self.open(packet).is_err() {
                self.closed = true;
                return Err(io::Error::from(ErrorKind::ConnectionAborted).into());
            }
            return Ok(());
        }
        let Some(ws) = self.ws.as_mut() else {
            return Err(io::Error::from(ErrorKind::ConnectionAborted).into());
        };
        match ws.send(Message::Binary(packet.to_vec().into())) {
            Ok(()) => Ok(()),
            // Non-blocking write that couldn't flush fully is buffered by tungstenite for next flush.
            Err(tungstenite::Error::Io(error)) if error.kind() == ErrorKind::WouldBlock => Ok(()),
            Err(error) => Err(io::Error::other(error.to_string()).into()),
        }
    }
}

// Mirrors the client's replicon<->renet2 bridge (game/client/src/core/net/transport.rs).
#[derive(Resource)]
struct Client(RenetClient);

#[derive(Resource)]
struct Transport(NetcodeClientTransport);

struct BridgePlugin;

impl bevy_app::Plugin for BridgePlugin {
    fn build(&self, app: &mut App) {
        use bevy_app::{PostUpdate, PreUpdate};
        app.add_systems(
            PreUpdate,
            (
                drive,
                set_connecting.run_if(in_state(ClientState::Disconnected).and_then(connecting)),
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

fn drive(
    client: Option<ResMut<Client>>,
    transport: Option<ResMut<Transport>>,
    time: Res<bevy_time::Time>,
) {
    let Some(mut client) = client else {
        return;
    };
    client.0.update(time.delta());
    if let Some(mut transport) = transport {
        let _ = transport.0.update(time.delta(), &mut client.0);
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
    mut client: ResMut<Client>,
    mut transport: ResMut<Transport>,
    mut messages: ResMut<ClientMessages>,
) {
    for (channel_id, message) in messages.drain_sent() {
        client.0.send_message(channel_id as u8, message);
    }
    let _ = transport.0.send_packets(&mut client.0);
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
