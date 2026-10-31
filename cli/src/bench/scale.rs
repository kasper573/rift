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
use engine::core::units::Bytes;
use engine::systems::GameSettings;

use super::client::settled_in_world;
use super::playtest::Playtest;
use super::sim::{self, Bench, Layout, PLAYERS_PER_AREA};
use super::tile::tiled;

/// The area's side, in sides of the spawn area: it grows by the square of these.
const SIDES: [u32; 6] = [1, 2, 3, 4, 6, 8];
const MEASURE: usize = 100;
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
        "[scale-bench] {:>5} {:>11} {:>6} {:>7} | {:>14} {:>9} | {:>14} {:>14} {:>9} | {:>6} {:>9} {:>8}",
        "areas",
        "ground",
        "npcs",
        "players",
        "one area: core",
        "heap",
        "many: one core",
        "every core",
        "heap",
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
        let one = server(1, players, &larger, Cores::One);
        let many_one_core = server(tiles, PLAYERS_PER_AREA, content, Cores::One);
        let many_every_core = server(tiles, PLAYERS_PER_AREA, content, Cores::Every);
        let seen = client(folder.path(), &larger);
        println!(
            "[scale-bench] {tiles:>5} {:>11} {:>6} {players:>7} | {:>11.2} ms {:>6.0} MB | {:>11.2} ms {:>11.2} ms {:>6.0} MB | {:>6.0} {:>6.0} MB {:>5.0} MB",
            format!("{}×{} m", spawn.size.width * side, spawn.size.height * side),
            super::npcs_per_area(&larger),
            one.tick_ms,
            one.heap.megabytes(),
            many_one_core.tick_ms,
            many_every_core.tick_ms,
            many_one_core.heap.megabytes(),
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
fn server(areas: usize, players: usize, content: &Content, cores: Cores) -> Served {
    let before = heap();
    let mut bench = Bench::with_players(areas, players, Layout::Distributed, content);
    bench.connect();
    for _ in 0..sim::WARMUP {
        step(&mut bench, cores);
    }
    let held = Bytes(heap().0.saturating_sub(before.0));
    let started = Instant::now();
    for _ in 0..MEASURE {
        step(&mut bench, cores);
    }
    Served {
        tick_ms: started.elapsed().as_secs_f64() * 1000.0 / MEASURE as f64,
        heap: held,
    }
}

fn step(bench: &mut Bench, cores: Cores) {
    let clock = bench.play();
    match cores {
        Cores::One => bench.tick_on(one_core(), clock),
        Cores::Every => bench.tick(clock),
    }
    bench.acknowledge_sent();
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
    let mut me = Playtest::start(game, content);
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
    engine::systems::load_content(
        std::sync::Arc::new(engine::core::content::FilesystemFolder(game.to_path_buf())),
        "the larger area did not load",
    )
    .map_err(|report| eprint!("{report}"))
    .ok()
}

/// What the process's allocator holds for live allocations.
fn heap() -> Bytes {
    tikv_jemalloc_ctl::epoch::advance().expect("jemalloc's statistics refresh");
    Bytes(tikv_jemalloc_ctl::stats::allocated::read().expect("jemalloc's statistics") as u64)
}
