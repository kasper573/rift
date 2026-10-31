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
use engine::core::units::Meters;
use engine::systems::{GameSettings, mirror};

use super::client::settled_in_world;
use super::playtest::Playtest;
use super::sim::{self, Bench, Layout, PLAYERS_PER_AREA};
use super::tile::{self, tiled};

/// The area's side, in sides of the spawn area: it grows by the square of these.
const SIDES: [u32; 6] = [1, 2, 3, 4, 6, 8];
const MEASURE: usize = 100;
/// A probe of the cell size is a few seconds of ticks: the sizes it tells apart differ by less
/// than a short run wavers.
const CELLS_MEASURE: usize = 2000;
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
        let one = server(1, players, &larger, Cores::One, MEASURE);
        let many_one_core = server(tiles, PLAYERS_PER_AREA, content, Cores::One, MEASURE);
        let many_every_core = server(tiles, PLAYERS_PER_AREA, content, Cores::Every, MEASURE);
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

/// The cell size at which the spawn area grown four by four, run as one area on one core, ticks
/// fastest, searched between the halo and the area's longest side: what a designer sets
/// `spatial_partition_cell_size` to for this game's content.
pub fn cells(game: &Path, content: &Content) {
    let settings = content.built::<GameSettings>();
    let spawn = settings.initial_spawn.area.get(content);
    let side = 4;
    let tiles = (side * side) as usize;
    let folder = tempfile::tempdir().expect("a folder for the larger game");
    if let Err(problem) = tiled(game, content, side, folder.path()) {
        println!("[cells-bench] not laid out: {problem}");
        return;
    }
    let players = PLAYERS_PER_AREA * tiles;
    let (halo, reached_by) = mirror::halo(content);
    println!(
        "[cells-bench] the spawn area laid out {side}×{side}, {}×{} m with {players} players, as one area on one core; worlds see {} m past their cell, as far as {reached_by}",
        spawn.size.width * side,
        spawn.size.height * side,
        halo.0
    );
    println!("[cells-bench] {:>8} | {:>10} {:>9}", "size", "tick", "heap");
    let mut probe = |candidate: Meters| -> Option<f64> {
        if let Err(problem) = tile::with_cell_size(folder.path(), content, candidate) {
            println!("[cells-bench] {:>6.0} m not set: {problem}", candidate.0);
            return None;
        }
        let larger = loaded(folder.path())?;
        let served = server(1, players, &larger, Cores::One, CELLS_MEASURE);
        println!(
            "[cells-bench] {:>6.0} m | {:>7.2} ms {:>6.0} MB",
            candidate.0,
            served.tick_ms,
            served.heap.megabytes()
        );
        Some(served.tick_ms)
    };
    let longest = (spawn.size.width.max(spawn.size.height) * side) as f32;
    let Some(best) = least(halo.0, longest, &mut probe) else {
        println!("[cells-bench] no size could be measured");
        return;
    };
    println!("[cells-bench] RESULT {best:.0} m: the spatial_partition_cell_size for this game");
}

/// The size in `low..=high` at which `tick` is least, by golden-section search: the tick rises
/// away from the best size on both hands, so the bracket closes on it, down to a tenth of the
/// size. A probe that can't be measured counts as the slowest.
fn least(low: f32, high: f32, tick: &mut impl FnMut(Meters) -> Option<f64>) -> Option<f32> {
    const GOLDEN: f32 = 0.618_034;
    let measure = |tick: &mut dyn FnMut(Meters) -> Option<f64>, size: f32| {
        tick(Meters(size.round())).unwrap_or(f64::INFINITY)
    };
    let (mut low, mut high) = (low, high);
    let mut inner = (high - (high - low) * GOLDEN, low + (high - low) * GOLDEN);
    let mut ticks = (measure(tick, inner.0), measure(tick, inner.1));
    while high - low > 0.1 * high {
        if ticks.0 < ticks.1 {
            high = inner.1;
            inner = (high - (high - low) * GOLDEN, inner.0);
            ticks = (measure(tick, inner.0), ticks.0);
        } else {
            low = inner.0;
            inner = (inner.1, low + (high - low) * GOLDEN);
            ticks = (ticks.1, measure(tick, inner.1));
        }
    }
    let best = if ticks.0 < ticks.1 { inner.0 } else { inner.1 };
    (ticks.0.min(ticks.1)).is_finite().then_some(best.round())
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
    let before = heap();
    let mut bench = Bench::with_players(areas, players, Layout::Distributed, content);
    bench.connect();
    for _ in 0..sim::WARMUP {
        step(&mut bench, cores);
    }
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
