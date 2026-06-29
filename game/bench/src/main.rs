mod assets;
#[cfg(feature = "profiling")]
mod profiling;
mod search;
mod sim;

use std::time::Instant;

use bevy_app::App;

const BUDGET_MS: f64 = 40.0;
// Must exceed the budget crossover (and the probe that overshoots it) but stay within RAM: the bench
// holds every area world in memory at once (~3 MiB each with clients), so this caps peak use near
// ~15 GiB. The search stops at the 40 ms crossover well before here anyway.
const MAX_AREAS: usize = 5000;
const WARMUP: usize = 30;
const MEASURE: usize = 200;

fn main() {
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
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().expect("cwd"));
        profiling::run(&out, layout);
        return;
    }

    if let Ok(spec) = std::env::var("BENCH_A") {
        for a in spec
            .split(',')
            .filter_map(|s| s.trim().parse::<usize>().ok())
        {
            let p = point(a, WARMUP, MEASURE, layout);
            println!(
                "[bench]   A={a:<4} full={:6.2}ms sim={:6.2}ms repl={:6.2}ms p99={:6.2}ms  {}",
                p.full,
                p.sim,
                (p.full - p.sim).max(0.0),
                p.p99,
                verdict(p.full),
            );
        }
        return;
    }

    println!(
        "[bench] finding the highest A sustained within the {BUDGET_MS:.0}ms budget ({} players)...",
        layout.label()
    );

    let mut best: Option<(usize, Point)> = None;
    let mut under: Option<(usize, f64)> = None;
    let mut over: Option<(usize, f64)> = None;
    let mut previous: Option<(usize, f64)> = None;
    let mut next = Some(1usize);
    while let Some(areas) = next {
        let p = point(areas, WARMUP, MEASURE, layout);
        let mean = p.full;
        println!(
            "[bench]   A={areas:<4} mean={mean:6.2}ms  {}",
            verdict(mean)
        );
        let last = (areas, mean);
        if mean <= BUDGET_MS {
            if under.is_none_or(|(highest, _)| areas >= highest) {
                under = Some(last);
                best = Some((areas, p));
            }
        } else if over.is_none_or(|(lowest, _)| areas <= lowest) {
            over = Some(last);
        }
        next = search::project(BUDGET_MS, MAX_AREAS, under, over, previous, last);
        previous = Some(last);
    }

    let (areas, r) = best.unwrap_or_else(|| (1, point(1, WARMUP, MEASURE, layout)));
    let npcs = sim::npcs_per_area() * areas;
    let players = sim::PLAYERS_PER_AREA * areas;
    println!("\n[bench] areas,npcs,players,clients,mean_ms,p50_ms,p99_ms,max_ms,sim_ms,repl_ms");
    println!(
        "[bench] RESULT {},{},{},{},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3}",
        areas,
        npcs,
        players,
        players,
        r.full,
        r.p50,
        r.p99,
        r.max,
        r.sim,
        (r.full - r.sim).max(0.0),
    );
    println!(
        "[bench] capacity: {} isolated areas = {} NPCs + {} players sustained at {:.1}ms/tick (budget {:.0}ms)",
        areas, npcs, players, r.full, BUDGET_MS,
    );
}

fn verdict(mean: f64) -> &'static str {
    if mean <= BUDGET_MS { "ok" } else { "over" }
}

struct Point {
    full: f64,
    sim: f64,
    p50: f64,
    p99: f64,
    max: f64,
}

fn point(areas: usize, warmup: usize, ticks: usize, layout: sim::Layout) -> Point {
    let assets = assets::service();
    let (mut worlds, rosters) = sim::worlds(areas, layout, &assets);

    let baseline = measure(&mut worlds, warmup, ticks);
    sim::connect(&mut worlds, &rosters);
    let full = measure(&mut worlds, warmup, ticks);

    Point {
        full: full.0,
        sim: baseline.0,
        p50: full.1,
        p99: full.2,
        max: full.3,
    }
}

fn measure(worlds: &mut [App], warmup: usize, ticks: usize) -> (f64, f64, f64, f64) {
    for _ in 0..warmup {
        sim::step(worlds);
    }
    let mut samples = Vec::with_capacity(ticks);
    for _ in 0..ticks {
        let started = Instant::now();
        sim::step(worlds);
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(f64::total_cmp);
    let n = samples.len();
    (
        samples.iter().sum::<f64>() / n as f64,
        samples[n / 2],
        samples[(n as f64 * 0.99) as usize],
        samples[n - 1],
    )
}
