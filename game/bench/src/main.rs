mod assets;
#[cfg(feature = "profiling")]
mod profiling;
mod sim;

use std::time::Instant;

use bevy_app::App;
use world::data;

const BUDGET_MS: f64 = 40.0;
const MAX_AREAS: usize = 768; // must exceed the crossover and the probe that overshoots it
const WARMUP: usize = 30;
const MEASURE: usize = 200;

fn main() {
    #[cfg(feature = "profiling")]
    if std::env::args().any(|arg| arg == "profile") {
        let out = std::env::args()
            .nth(2)
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().expect("cwd"));
        profiling::run(&out);
        return;
    }

    println!("[bench] finding the highest A sustained within the {BUDGET_MS:.0}ms budget...");

    let mut best: Option<(usize, Point)> = None;
    let mut under: Option<(usize, f64)> = None;
    let mut over: Option<(usize, f64)> = None;
    let mut previous: Option<(usize, f64)> = None;
    let mut next = Some(1usize);
    while let Some(areas) = next {
        let p = point(areas, WARMUP, MEASURE);
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
        next = project_areas(under, over, previous, last);
        previous = Some(last);
    }

    let (areas, r) = best.unwrap_or_else(|| (1, point(1, WARMUP, MEASURE)));
    let npcs = sim::NPCS_PER_AREA * areas;
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

/// The next area count to probe while root-finding the budget crossover, or `None` to stop. Stops once
/// the projected next probe is only one area past the best sustained one — a finer answer isn't worth a
/// probe. With the budget bracketed it uses false position; before that it leaps straight at the budget
/// (secant of the last two probes, else one proportional guess), so a fast world is bracketed at once.
fn project_areas(
    under: Option<(usize, f64)>,
    over: Option<(usize, f64)>,
    previous: Option<(usize, f64)>,
    last: (usize, f64),
) -> Option<usize> {
    let (ua, ut) = under?;
    if ua >= MAX_AREAS {
        return None;
    }
    let next = match over {
        Some((oa, ot)) => {
            if oa <= ua + 1 {
                return None;
            }
            let guess = if ot > ut {
                ua as f64 + (BUDGET_MS - ut) * (oa - ua) as f64 / (ot - ut)
            } else {
                (ua + oa) as f64 / 2.0
            };
            (guess.round() as usize).clamp(ua + 1, oa - 1)
        }
        None => {
            let projected = match previous {
                Some((pa, pt)) if last.0 != pa && last.1 > pt => {
                    let slope = (last.1 - pt) / (last.0 - pa) as f64;
                    last.0 as f64 + (BUDGET_MS - last.1) / slope
                }
                _ => ua as f64 * BUDGET_MS / ut,
            };
            (projected.round() as usize).clamp(ua + 1, MAX_AREAS)
        }
    };
    (next > ua + 1).then_some(next)
}

struct Point {
    full: f64,
    sim: f64,
    p50: f64,
    p99: f64,
    max: f64,
}

fn point(areas: usize, warmup: usize, ticks: usize) -> Point {
    let npc = data::npc::Id::Orc;
    let assets = assets::service();
    let (mut worlds, rosters) = sim::worlds(areas, npc, &assets);

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
