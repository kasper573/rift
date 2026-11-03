//! Load generator: connects headless clients to an already-running real server over the client's
//! own transport and holds them idle. The server must take any account's sign-in (its auth
//! `fake`). Without a count it root-finds the player count whose server tick hits the budget,
//! using the same projection as the area benchmark.

use std::time::{Duration, Instant};

use bevy_app::{App, Update};
use bevy_ecs::prelude::*;
use engine::core::content::sync::{MANIFEST, Manifest};
use engine::core::net::SessionRequest;
use engine::core::net::transport::{ClientTransportPlugin, Connection, read_token};
use engine::systems::TICK_HZ;
use engine::systems::launch::{TICK_METRIC, fake_token};
use engine::systems::player::JoinRequest;
use engine::systems::player::session::{self, ClientSessionPlugin};

use crate::search::{Search, budget_ms, verdict};

/// Search clamp for the player root-find; the real limit is the server's client cap or its tick time.
const MAX_PLAYERS: usize = 50_000;
/// Open this many connections before pumping, so already-connected clients keep draining while we connect.
const CONNECT_BATCH: usize = 100;
/// Let the server settle at a new player count before sampling its tick.
const STABILIZE: Duration = Duration::from_secs(2);
/// Sample the server tick over this window.
const WINDOW: Duration = Duration::from_secs(3);

pub fn run(http_url: &str, count: Option<usize>) -> Result<(), String> {
    match count {
        None => run_dyn(http_url).map(drop),
        Some(count) => run_fixed(http_url, count),
    }
}

fn run_fixed(http_url: &str, count: usize) -> ! {
    println!("[loadtest] connecting {count} clients to {http_url} (fake tokens)");
    let started = Instant::now();
    let mut clients: Vec<App> = Vec::new();
    let mut next_id = 0usize;
    set_player_count(&mut clients, http_url, &mut next_id, count);
    println!(
        "[loadtest] opened {}/{count} in {:.1}s — holding (Ctrl-C to stop)",
        connected_count(&clients),
        started.elapsed().as_secs_f64()
    );
    let mut report = Instant::now();
    loop {
        pump(&mut clients, TICK_HZ.period());
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
pub fn run_dyn(http_url: &str) -> Result<Pool, String> {
    println!(
        "[loadtest] root-finding the {:.1}ms/tick player equilibrium ({http_url})",
        budget_ms()
    );
    let mut clients: Vec<App> = Vec::new();
    let mut next_id = 0usize;
    let mut search = Search::up_to(MAX_PLAYERS);
    let mut next = Some(1usize);
    while let Some(target) = next {
        set_player_count(&mut clients, http_url, &mut next_id, target);
        let live = connected_count(&clients);
        let ms = measure_tick_ms(http_url, &mut clients, WINDOW)?;
        println!(
            "[loadtest]   {live:>6} players  →  server {ms:6.2}ms/tick  {}",
            verdict(ms)
        );

        next = search.next_after(live, ms);
        // Couldn't OPEN as many connections as asked (not merely "not all connected yet") while still
        // under budget: the limit is the machine's or server's connection ceiling, not tick time, and
        // that count is the answer. Compare opened connections, not `live`, so a slow handshake isn't
        // mistaken for a ceiling.
        if clients.len() < target && ms < budget_ms() {
            println!("[loadtest] connection ceiling at {live} players (could not open more)");
            break;
        }
    }
    match search.sustained() {
        Some((players, ms)) => {
            println!(
                "[loadtest] equilibrium: ~{players} concurrent players sustained at {ms:.1}ms/tick (budget {:.1}ms)",
                budget_ms()
            );
            set_player_count(&mut clients, http_url, &mut next_id, players);
        }
        None => println!("[loadtest] the first probe already exceeded the budget"),
    }
    Ok(Pool(clients))
}

/// Simulated players, connected for as long as it's kept and pumped.
pub struct Pool(Vec<App>);

impl Pool {
    /// Keeps every client answering the server for `lasting`.
    pub fn hold(&mut self, lasting: Duration) {
        pump(&mut self.0, lasting);
    }
}

/// Brings the live client pool to exactly `target` players: drops surplus clients (each dropped
/// connection says goodbye, which the server sees as a disconnect) or opens new ones. New clients
/// connect in batches, pumping the pool between batches so already-connected ones keep draining; a batch that
/// opens nothing means a connection ceiling was hit. Fake ids are monotonic so a reconnecting slot is
/// never mistaken for a still-connected account.
fn set_player_count(clients: &mut Vec<App>, http_url: &str, next_id: &mut usize, target: usize) {
    clients.truncate(target);
    while clients.len() < target {
        let want = CONNECT_BATCH.min(target - clients.len());
        let before = clients.len();
        for _ in 0..want {
            let id = *next_id;
            *next_id += 1;
            if let Some(app) = build_client(http_url, id) {
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
        if let Some(remaining) = TICK_HZ.period().checked_sub(frame.elapsed()) {
            std::thread::sleep(remaining);
        }
        if Instant::now() >= deadline {
            return;
        }
    }
}

fn measure_tick_ms(http_url: &str, clients: &mut [App], window: Duration) -> Result<f64, String> {
    let before = scrape_tick(http_url);
    pump(clients, window);
    let after = scrape_tick(http_url);
    match (before, after) {
        (Some((sum0, count0)), Some((sum1, count1))) if count1 > count0 => {
            Ok((sum1 - sum0) / (count1 - count0) * 1000.0)
        }
        _ => Err(format!(
            "{http_url}/metrics did not report the server ticking over the window"
        )),
    }
}

fn scrape_tick(http_url: &str) -> Option<(f64, f64)> {
    let body = ureq::get(format!("{http_url}/metrics"))
        .call()
        .ok()?
        .body_mut()
        .read_to_string()
        .ok()?;
    let reading = |name: String| {
        body.lines()
            .find(|line| line.starts_with(&name))?
            .rsplit(' ')
            .next()?
            .parse::<f64>()
            .ok()
    };
    Some((
        reading(format!("{TICK_METRIC}_sum"))?,
        reading(format!("{TICK_METRIC}_count"))?,
    ))
}

fn connected_count(clients: &[App]) -> usize {
    clients
        .iter()
        .filter(|app| {
            app.world()
                .get_resource::<Connection>()
                .is_some_and(Connection::is_connected)
        })
        .count()
}

fn build_client(http_url: &str, index: usize) -> Option<App> {
    let token = fetch_token(http_url, &format!("loadtest-{index}"))?;
    let connection = Connection::open(read_token(&token)?).ok()?;

    let mut app = App::new();
    app.add_plugins(bevy_time::TimePlugin);
    app.add_plugins((ClientSessionPlugin, ClientTransportPlugin));
    app.init_resource::<Joined>();
    app.add_systems(Update, announce_join);
    app.insert_resource(connection);
    app.finish();
    app.cleanup();
    Some(app)
}

fn fetch_token(http_url: &str, account: &str) -> Option<String> {
    let manifest = ureq::get(format!("{http_url}{MANIFEST}"))
        .call()
        .ok()?
        .body_mut()
        .read_to_string()
        .ok()?;
    let manifest: Manifest = serde_json::from_str(&manifest).ok()?;
    let request = SessionRequest {
        build: manifest.build,
        rows: manifest.rows.hash,
    };
    let mut response = ureq::post(format!("{http_url}/session"))
        .header("Authorization", format!("Bearer {}", fake_token(account)))
        .header("Content-Type", "application/json")
        .send(serde_json::to_string(&request).ok()?)
        .ok()?;
    response.body_mut().read_to_string().ok()
}

// Once the server's welcome arrives (`MyClient` set), announce the join exactly as the real client
// does — without it the client connects but no player character is ever spawned server-side, so the
// simulation runs with zero real players.
#[derive(Resource, Default)]
struct Joined(bool);

fn announce_join(world: &mut World) {
    if world.resource::<Joined>().0 || session::my_id(world).is_none() {
        return;
    }
    world.write_message(JoinRequest);
    world.resource_mut::<Joined>().0 = true;
}
