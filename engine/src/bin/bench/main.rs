mod play;
#[cfg(feature = "profiling")]
mod profiling;
mod search;
mod sim;

use std::alloc::{GlobalAlloc, Layout};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::Instant;

use bevy_app::App;
use engine::core::content::{Content, FilesystemFolder};
use engine::systems::load_content;
use serde::Deserialize;

use crate::search::{BUDGET_MS, Search};

// The server's own allocator setup, so the measured ticks allocate exactly as production does.
service::heap_profiling!(Counting, Counting);

static ALLOCS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);

struct Counting;

// SAFETY: every call goes unchanged to the services' allocator, which upholds GlobalAlloc's
// contract; counting touches only atomics and never allocates.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed);
        BYTES.fetch_add(layout.size() as u64, Relaxed);
        unsafe { service::ServiceAllocator.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { service::ServiceAllocator.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed);
        if new_size > layout.size() {
            BYTES.fetch_add((new_size - layout.size()) as u64, Relaxed);
        }
        unsafe { service::ServiceAllocator.realloc(ptr, layout, new_size) }
    }
}

// Must exceed the budget crossover (and the probe that overshoots it) but stay within RAM: the bench
// holds every area world in memory at once (~3 MiB each with clients), so this caps peak use near
// ~15 GiB. The search stops at the 40 ms crossover well before here anyway.
const MAX_AREAS: usize = 5000;
const MEASURE: usize = 200;

#[derive(Deserialize)]
struct Config {
    content_dir: PathBuf,
    a: Option<Vec<usize>>,
}

fn main() {
    let config: Config = envy::prefixed("RIFT_BENCH_")
        .from_env()
        .expect("RIFT_BENCH_* environment");
    // `dist` selects distributed (spread-out) players; the default is congested (all on the spawn
    // tile, sharing one view).
    let layout = if std::env::args().any(|arg| arg == "dist") {
        sim::Layout::Distributed
    } else {
        sim::Layout::Congested
    };

    #[cfg(feature = "profiling")]
    if std::env::args().any(|arg| arg == "profile") {
        let out = std::env::args()
            .skip(1)
            .find(|arg| arg != "profile" && arg != "dist")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().expect("cwd"));
        profiling::run(&out, layout, load(&config.content_dir));
        return;
    }

    if let Some(probes) = config.a {
        for a in probes {
            let p = point(a, layout, &config.content_dir);
            println!(
                "[bench]   A={a:<4} full={:6.2}ms sim={:6.2}ms repl={:6.2}ms p99={:6.2}ms  {}",
                p.full.mean,
                p.sim,
                (p.full.mean - p.sim).max(0.0),
                p.full.p99,
                verdict(p.full.mean),
            );
        }
        return;
    }

    println!(
        "[bench] finding the highest A sustained within the {BUDGET_MS:.0}ms budget ({} players)...",
        layout.as_ref()
    );

    let mut search = Search::up_to(MAX_AREAS);
    let mut best: Option<(usize, Point)> = None;
    let mut next = Some(1usize);
    while let Some(areas) = next {
        let p = point(areas, layout, &config.content_dir);
        let mean = p.full.mean;
        println!(
            "[bench]   A={areas:<4} mean={mean:6.2}ms  {}",
            verdict(mean)
        );
        next = search.next_after(areas, mean);
        if search
            .sustained()
            .is_some_and(|(highest, _)| highest == areas)
        {
            best = Some((areas, p));
        }
    }

    let (areas, r) = best.unwrap_or_else(|| (1, point(1, layout, &config.content_dir)));
    let npcs = sim::npcs_per_area(&load(&config.content_dir)) * areas;
    let players = sim::PLAYERS_PER_AREA * areas;
    println!("\n[bench] areas,npcs,players,clients,mean_ms,p50_ms,p99_ms,max_ms,sim_ms,repl_ms");
    println!(
        "[bench] RESULT {},{},{},{},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3}",
        areas,
        npcs,
        players,
        players,
        r.full.mean,
        r.full.p50,
        r.full.p99,
        r.full.max,
        r.sim,
        (r.full.mean - r.sim).max(0.0),
    );
    println!(
        "[bench] capacity: {} isolated areas = {} NPCs + {} players sustained at {:.1}ms/tick (budget {:.0}ms)",
        areas, npcs, players, r.full.mean, BUDGET_MS,
    );
}

fn load(content_dir: &Path) -> Content {
    load_content(
        Arc::new(FilesystemFolder(content_dir.to_path_buf())),
        "the program stopped",
    )
    .unwrap_or_else(|report| {
        eprint!("{report}");
        std::process::exit(2);
    })
}

fn verdict(mean: f64) -> &'static str {
    if mean <= BUDGET_MS { "ok" } else { "over" }
}

struct Point {
    full: Timings,
    sim: f64,
}

struct Timings {
    mean: f64,
    p50: f64,
    p99: f64,
    max: f64,
}

fn point(areas: usize, layout: sim::Layout, content_dir: &Path) -> Point {
    let content = load(content_dir);
    let (mut worlds, rosters) = sim::worlds(areas, layout, &content);

    let sim = measure(&mut worlds, &rosters, &content).mean;
    sim::connect(&mut worlds, &rosters);
    Point {
        full: measure(&mut worlds, &rosters, &content),
        sim,
    }
}

fn measure(worlds: &mut [App], rosters: &[sim::Roster], content: &Content) -> Timings {
    for _ in 0..sim::WARMUP {
        sim::step(worlds, rosters, content);
    }
    let mut samples = Vec::with_capacity(MEASURE);
    for _ in 0..MEASURE {
        let started = Instant::now();
        sim::step(worlds, rosters, content);
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(f64::total_cmp);
    let n = samples.len();
    Timings {
        mean: samples.iter().sum::<f64>() / n as f64,
        p50: samples[n / 2],
        p99: samples[(n as f64 * 0.99) as usize],
        max: samples[n - 1],
    }
}
