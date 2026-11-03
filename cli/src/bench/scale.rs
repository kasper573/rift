//! How the server and the client fare as an area grows: the game's spawn area laid out again and
//! again into one larger area, everything its scene holds coming along at the same density, against
//! as many separate areas holding the same. The server steps the one larger area as it steps the
//! many, each with as many players to its ground, measured on one core and on every core, with the
//! memory each holds. The client then plays in the larger area, at the same spot of the first tile.
//! An area grows until the engine refuses to load one that large.

use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use engine::core::content::Content;
use engine::core::render::video::VideoPreferences;
use engine::core::units::Bytes;
use engine::systems::GameSettings;

use super::client::settled_in_world;
use super::playtest::Playtest;
use super::sim::{self, Bench, Layout, PLAYERS_PER_AREA};
use super::tile::tiled;

/// The area's side, in sides of the spawn area: it grows by the square of these.
const SIDES: [u32; 6] = [1, 2, 3, 4, 6, 8];
const MEASURE: usize = 100;
/// How long each size is watched: a few seconds of ticks, as the sizes differ by less than a
/// short run wavers.
const WATCH: Duration = Duration::from_secs(4);

pub fn run(game: &Path, content: &Content) {
    let spawn = content
        .built::<GameSettings>()
        .initial_spawn
        .area
        .get(content);
    println!(
        "[scale-bench] the spawn area, {}×{} m, laid out side by side; {PLAYERS_PER_AREA} players to each",
        spawn.size.width, spawn.size.height
    );
    println!(
        "[scale-bench] {:>5} {:>11} {:>6} {:>7} | {:>14} {:>10} {:>9} | {:>14} {:>10} {:>9} | {:>11} {:>11} {:>11} | {:>6} {:>9} {:>8}",
        "areas",
        "ground",
        "npcs",
        "players",
        "one area: core",
        "every core",
        "heap",
        "many: one core",
        "every core",
        "heap",
        "25% dormant",
        "50% dormant",
        "75% dormant",
        "fps",
        "client",
        "gpu"
    );
    for side in SIDES {
        let tiles = (side * side) as usize;
        let folder = tempfile::tempdir().expect("a folder for the larger game");
        if let Err(problem) = tiled(game, content, side, folder.path()) {
            println!("[scale-bench] {tiles:>5} not laid out: {problem}");
            return;
        }
        let Some(larger) = loaded(folder.path()) else {
            println!(
                "[scale-bench] {tiles:>5} refused: the engine doesn't load an area {}×{} m",
                spawn.size.width * side,
                spawn.size.height * side
            );
            return;
        };
        let players = PLAYERS_PER_AREA * tiles;
        let one = server(1, players, &larger, Cores::One, MEASURE);
        let one_every_core = server(1, players, &larger, Cores::Every, MEASURE);
        let many_one_core = server(tiles, PLAYERS_PER_AREA, content, Cores::One, MEASURE);
        let many_every_core = server(tiles, PLAYERS_PER_AREA, content, Cores::Every, MEASURE);
        let dormant: Vec<f64> = [0.25, 0.5, 0.75]
            .into_iter()
            .map(|dormant| {
                let awake = 1.0 - dormant;
                served_in(
                    1,
                    (players as f32 * awake).ceil() as usize,
                    &larger,
                    Cores::Every,
                    MEASURE,
                    awake,
                )
                .tick_ms
            })
            .collect();
        let seen = client(folder.path(), &larger);
        println!(
            "[scale-bench] {tiles:>5} {:>11} {:>6} {players:>7} | {:>11.2} ms {:>7.2} ms {:>6.0} MB | {:>11.2} ms {:>7.2} ms {:>6.0} MB | {:>8.2} ms {:>8.2} ms {:>8.2} ms | {:>6.0} {:>6.0} MB {:>5.0} MB",
            format!("{}×{} m", spawn.size.width * side, spawn.size.height * side),
            super::npcs_per_area(&larger),
            one.tick_ms,
            one_every_core.tick_ms,
            one.heap.megabytes(),
            many_one_core.tick_ms,
            many_every_core.tick_ms,
            many_one_core.heap.megabytes(),
            dormant[0],
            dormant[1],
            dormant[2],
            seen.fps,
            seen.client.megabytes(),
            seen.gpu.megabytes(),
        );
    }
}

#[derive(Clone, Copy)]
enum Cores {
    One,
    Every,
}

struct Served {
    tick_ms: f64,
    heap: Bytes,
}

/// The mean tick of `areas` of the content's spawn area with `players` each, and the heap they
/// hold.
fn server(areas: usize, players: usize, content: &Content, cores: Cores, ticks: usize) -> Served {
    served_in(areas, players, content, cores, ticks, 1.0)
}

/// As [`server`], with the players in the first `awake` share of each area's ground alone.
fn served_in(
    areas: usize,
    players: usize,
    content: &Content,
    cores: Cores,
    ticks: usize,
    awake: f32,
) -> Served {
    let before = heap();
    let mut bench = Bench::with_players_in(areas, players, Layout::Distributed, content, awake);
    bench.connect();
    sim::warm_up(&mut bench, |bench| step(bench, cores));
    let held = Bytes(heap().0.saturating_sub(before.0));
    let started = Instant::now();
    for _ in 0..ticks {
        step(&mut bench, cores);
    }
    Served {
        tick_ms: started.elapsed().as_secs_f64() * 1000.0 / ticks as f64,
        heap: held,
    }
}

fn step(bench: &mut Bench, cores: Cores) {
    let clock = bench.play();
    match cores {
        Cores::One => bench.tick(Some(one_core()), clock),
        Cores::Every => bench.tick(None, clock),
    }
    bench.send_out();
}

fn one_core() -> &'static rayon::ThreadPool {
    static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .expect("a one-thread pool")
    })
}

struct Seen {
    fps: f64,
    client: Bytes,
    gpu: Bytes,
}

/// The client's frame rate and memory, standing where the game starts.
fn client(game: &Path, content: &Content) -> Seen {
    let mut me = Playtest::start(game, content, VideoPreferences::default());
    settled_in_world(&mut me);
    let first = me.snapshot().frame;
    let since = Instant::now();
    me.linger(WATCH);
    let last = me.snapshot();
    Seen {
        fps: f64::from(last.frame - first) / since.elapsed().as_secs_f64(),
        client: me.client_memory(),
        gpu: last.gpu_memory,
    }
}

fn loaded(game: &Path) -> Option<Content> {
    crate::folder::load(
        game,
        "the larger area did not load",
        engine::core::content::LoadPurpose::Play,
    )
    .map_err(|report| eprint!("{report}"))
    .ok()
}

/// What the process's allocator holds for live allocations.
fn heap() -> Bytes {
    Bytes(service::allocated_bytes().expect("the bench's allocator reports what it holds"))
}
