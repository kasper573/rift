mod client;
mod desktop;
mod memory;
mod play;
mod playtest;
mod sim;

use std::path::Path;
use std::time::Instant;

use engine::core::content::Content;
use engine::systems::area::{self, AreaModule};
use engine::systems::{GameSettings, step_areas};

use crate::search::{BUDGET_MS, Search};
use sim::{Bench, load};

// Must exceed the budget crossover (and the probe that overshoots it) but stay within RAM: the bench
// holds every area world in memory at once (~3 MiB each with clients), so this caps peak use near
// ~15 GiB. The search stops at the 40 ms crossover well before here anyway.
const MAX_AREAS: usize = 5000;
const MEASURE: usize = 200;

/// A part of the bench on a game.
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum BenchPart {
    /// The server's capacity.
    Server,
    /// The desktop client's frame rate.
    Client,
    /// Whether the client and the server hold their memory level as a player plays.
    Memory,
}

/// The server bench on the crowd, or on `game` followed by the client and memory benches, or only
/// the parts named. Fails naming what the memory bench saw grow.
pub fn run(game: Option<&Path>, spread: bool, only: &[BenchPart]) -> Result<(), String> {
    let runs = |part| only.is_empty() || only.contains(&part);
    let layout = if spread {
        sim::Layout::Distributed
    } else {
        sim::Layout::Congested
    };
    match game {
        None if only.iter().any(|&part| part != BenchPart::Server) => {
            Err("the client and memory benches play a game: name one".to_owned())
        }
        None => {
            let folder = tempfile::tempdir().expect("a folder to write the crowd into");
            testkit::crowd().write_to(folder.path());
            server(&load(folder.path()), layout);
            Ok(())
        }
        Some(game) => {
            let content = load(game);
            if runs(BenchPart::Server) {
                server(&content, layout);
            }
            if runs(BenchPart::Client) {
                client::run(game, &content);
            }
            if runs(BenchPart::Memory) {
                memory::run(game, &content)
            } else {
                Ok(())
            }
        }
    }
}

/// The highest count of the game's spawn area, each with its players, the server sustains within
/// the tick budget.
fn server(content: &Content, layout: sim::Layout) {
    println!(
        "[bench] finding the highest A sustained within the {BUDGET_MS:.0}ms budget ({} players)...",
        layout.as_ref()
    );

    let mut search = Search::up_to(MAX_AREAS);
    let mut best: Option<(usize, Point)> = None;
    let mut next = Some(1usize);
    while let Some(areas) = next {
        let p = point(areas, layout, content);
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

    let (areas, r) = best.unwrap_or_else(|| (1, point(1, layout, content)));
    let npcs = npcs_per_area(content) * areas;
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

fn point(areas: usize, layout: sim::Layout, content: &Content) -> Point {
    let mut bench = Bench::new(areas, layout, content);
    let sim = measure(&mut bench).mean;
    bench.connect();
    Point {
        full: measure(&mut bench),
        sim,
    }
}

fn measure(bench: &mut Bench) -> Timings {
    for _ in 0..sim::WARMUP {
        step(bench);
    }
    let mut samples = Vec::with_capacity(MEASURE);
    for _ in 0..MEASURE {
        let started = Instant::now();
        step(bench);
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

/// Advances every world one tick across all cores — the same parallel area fan-out the real server
/// uses, so the benchmark's capacity reflects the threaded server.
fn step(bench: &mut Bench) {
    let clock = bench.play();
    step_areas(&mut bench.worlds, clock);
    bench.acknowledge_sent();
}

fn npcs_per_area(content: &Content) -> usize {
    let area = content.built::<GameSettings>().initial_spawn.area;
    area::modules(content, area)
        .map(|module| match module {
            AreaModule::Population(population) => population.count as usize,
            AreaModule::Resident(_) => 1,
            _ => 0,
        })
        .sum()
}
