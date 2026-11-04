mod desktop;
mod driver;
mod editor;
mod frames;
mod memory;
mod places;
mod play;
mod playtest;
mod report;
mod scale;
mod scenario;
mod scenarios;
mod sim;
mod tile;

use std::path::{Path, PathBuf};
use std::time::Instant;

use engine::core::content::Content;
use engine::systems::GameSettings;
use engine::systems::area::{self, AreaModule};

use crate::search::{Search, budget_ms, verdict};
use driver::{Driver, Failure};
pub use frames::Video as VideoPreset;
use frames::{Criteria, Video};
use report::{BenchReport, Resolution, ScenarioResult, Verdict};
use scenario::{Scenario, Target};
use scenarios::ScenarioSlots;
use sim::{Bench, load};

// Must exceed the budget crossover (and the probe that overshoots it) but stay within RAM: the bench
// holds every area world in memory at once (~3 MiB each with clients), so this caps peak use near
// ~15 GiB. The search stops at the budget crossover well before here anyway.
const MAX_AREAS: usize = 5000;
const MEASURE: usize = 200;

/// A part of the bench on a game.
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum BenchPart {
    /// The server's capacity.
    Server,
    /// The desktop client's frame rate through its scenarios.
    Client,
    /// The editor's frame rate on every area, seen whole and close up.
    Editor,
    /// Whether the client and the server hold their memory level as a player plays.
    Memory,
    /// How the server and the client fare as the spawn area grows, against as many areas holding
    /// the same; run only when named, as it plays the game at every size.
    Scale,
}

/// How a bench is asked for.
pub struct BenchArgs {
    pub game: Option<PathBuf>,
    pub spread: bool,
    pub only: Vec<BenchPart>,
    /// Only the scenarios whose names match, `*` standing for anything.
    pub scenario: Option<String>,
    /// The video presets to run at, both unless given.
    pub video: Option<Video>,
    /// How many times each scenario runs; the report keeps the median run and the spread.
    pub repeat: u32,
    pub report: PathBuf,
    pub compare: Option<PathBuf>,
    /// The adapter the run must be on, refused on any other.
    pub adapter: Option<String>,
    /// One short scenario per target, proving the harness alone, without criteria.
    pub smoke: bool,
}

/// The server bench on the crowd, or on `game` followed by the client, editor and memory benches,
/// or only the parts named. The exit code: 0 when everything passed, 1 when a scenario fell short
/// of its criteria or the comparison got worse, 2 when a scenario could not be played through, 3
/// when a process died.
pub fn run(args: &BenchArgs) -> Result<i32, String> {
    let runs = |part| {
        (args.only.is_empty() && !matches!(part, BenchPart::Scale)) || args.only.contains(&part)
    };
    let layout = if args.spread {
        sim::Layout::Distributed
    } else {
        sim::Layout::Congested
    };
    let Some(game) = args.game.as_deref() else {
        if args.only.iter().any(|&part| part != BenchPart::Server) {
            return Err("the client, editor and memory benches play a game: name one".to_owned());
        }
        let folder = tempfile::tempdir().expect("a folder to write the crowd into");
        testkit::crowd().write_to(folder.path());
        server(&load(folder.path()), layout);
        return Ok(0);
    };
    let content = load(game);
    if runs(BenchPart::Server) {
        server(&content, layout);
    }
    let videos: Vec<Video> = match args.video {
        Some(video) => vec![video],
        None => Video::ALL.to_vec(),
    };
    let mut report: Option<BenchReport> = None;
    if runs(BenchPart::Client) {
        client(game, &content, args, &videos, &mut report)?;
    }
    if runs(BenchPart::Editor) {
        let results = editor::run(game, &content, &videos, args.scenario.as_deref());
        let report = report.get_or_insert_with(|| {
            BenchReport::new(
                args.adapter.clone().unwrap_or_default(),
                Resolution {
                    width: desktop::SCREEN.0,
                    height: desktop::SCREEN.1,
                },
                game,
            )
        });
        for result in results {
            report.add(result);
        }
    }
    if runs(BenchPart::Scale) {
        scale::run(game, &content);
    }
    let mut exit = 0;
    if runs(BenchPart::Memory) {
        memory::run(game, &content)?;
    }
    if let Some(report) = &report {
        report.write(&args.report)?;
        println!("[bench] report written to {}", args.report.display());
        exit = exit.max(report.overall.exit_code());
        if let Some(before) = &args.compare {
            let before = BenchReport::read(before)?;
            let comparison = report.compare(&before);
            for row in &comparison.rows {
                println!("[bench] {row}");
            }
            if comparison.worse.is_empty() {
                println!(
                    "[bench] nothing got worse against build {}",
                    before.engine_build
                );
            } else {
                for worse in &comparison.worse {
                    println!("[bench] WORSE {worse}");
                }
                exit = exit.max(1);
            }
        }
    }
    Ok(exit)
}

/// Every client scenario at each of `videos`, on a client of its own per preset, restarted when
/// one dies.
fn client(
    game: &Path,
    content: &Content,
    args: &BenchArgs,
    videos: &[Video],
    report: &mut Option<BenchReport>,
) -> Result<(), String> {
    let slots = ScenarioSlots::of(game, content)?;
    let scenarios: Vec<Scenario> = scenarios::client::catalogue(&slots, args.smoke)
        .into_iter()
        .filter(|scenario| {
            args.scenario
                .as_deref()
                .is_none_or(|only| crate::compare::matches(only, &scenario.name))
        })
        .collect();
    if scenarios.is_empty() {
        println!("[client-bench] no scenario to run");
        return Ok(());
    }
    let folder = tempfile::tempdir().map_err(|error| error.to_string())?;
    for &video in videos {
        let mut driver = Driver::start(game, content, video, folder.path())
            .map_err(|failure| failure_text(&failure))?;
        let adapter = driver
            .adapter()
            .map_err(|failure| failure_text(&failure))?
            .unwrap_or_default();
        if let Some(wanted) = &args.adapter
            && wanted != &adapter
        {
            return Err(format!(
                "the client draws on {adapter}, not the {wanted} the bench was asked for"
            ));
        }
        let screen = driver.screen().map_err(|failure| failure_text(&failure))?;
        let report = report.get_or_insert_with(|| {
            BenchReport::new(
                adapter.clone(),
                Resolution {
                    width: screen.width as u32,
                    height: screen.height as u32,
                },
                game,
            )
        });
        for scenario in &scenarios {
            let result = measure(
                &mut driver,
                scenario,
                video,
                args,
                game,
                content,
                folder.path(),
            );
            print_result(&result);
            report.add(result);
            if driver_dead(&mut driver) {
                driver = Driver::start(game, content, video, folder.path())
                    .map_err(|failure| failure_text(&failure))?;
            }
        }
    }
    Ok(())
}

fn driver_dead(driver: &mut Driver) -> bool {
    driver.adapter().is_err()
}

/// The scenario run `repeat` times, the median run by its mean kept, with how far the others
/// strayed from it.
fn measure(
    driver: &mut Driver,
    scenario: &Scenario,
    video: Video,
    args: &BenchArgs,
    game: &Path,
    content: &Content,
    folder: &Path,
) -> ScenarioResult {
    let criteria = scenario.criteria.map_or(Criteria::global(video), |own| {
        Criteria::global(video).stricter(own)
    });
    let mut results = Vec::new();
    for run in 0..args.repeat.max(1) {
        let started = Instant::now();
        let outcome = driver.run(scenario);
        let result = match outcome {
            Ok(measured) => {
                let stats = measured.window.stats(&criteria);
                let failures = match (&stats, args.smoke) {
                    (Some(stats), false) => criteria.failures(stats),
                    (None, _) => vec!["no frames were measured".to_owned()],
                    (Some(_), true) => Vec::new(),
                };
                let headroom = scenario.class.headroom(video);
                ScenarioResult {
                    name: scenario.name.clone(),
                    target: scenario.target,
                    class: scenario.class,
                    video,
                    verdict: if failures.is_empty() {
                        Verdict::Passed
                    } else {
                        Verdict::Criteria
                    },
                    failures,
                    headroom_met: match (&stats, headroom) {
                        (Some(stats), Some(headroom)) => Some(stats.mean.0 >= headroom.0),
                        _ => None,
                    },
                    headroom,
                    stats,
                    time_to_stand_frames: measured.stood_after_frames,
                    runs: 1,
                    mean_spread: None,
                }
            }
            Err(failure) => ScenarioResult {
                name: scenario.name.clone(),
                target: scenario.target,
                class: scenario.class,
                video,
                verdict: match failure {
                    Failure::Harness(_) => Verdict::Harness,
                    Failure::Crashed(_) => Verdict::Crashed,
                },
                failures: vec![failure_text(&failure)],
                stats: None,
                headroom: scenario.class.headroom(video),
                headroom_met: None,
                time_to_stand_frames: Vec::new(),
                runs: 1,
                mean_spread: None,
            },
        };
        println!(
            "[client-bench] {:<8} {:<24} run {} of {} in {:.0} s",
            video.name(),
            scenario.name,
            run + 1,
            args.repeat.max(1),
            started.elapsed().as_secs_f64()
        );
        let crashed = result.verdict == Verdict::Crashed;
        results.push(result);
        if crashed && run + 1 < args.repeat.max(1) {
            match Driver::start(game, content, video, folder) {
                Ok(fresh) => *driver = fresh,
                Err(failure) => {
                    results.push(ScenarioResult {
                        name: scenario.name.clone(),
                        target: scenario.target,
                        class: scenario.class,
                        video,
                        verdict: Verdict::Crashed,
                        failures: vec![failure_text(&failure)],
                        stats: None,
                        headroom: None,
                        headroom_met: None,
                        time_to_stand_frames: Vec::new(),
                        runs: 1,
                        mean_spread: None,
                    });
                    break;
                }
            }
        }
    }
    median_run(results)
}

/// Of several runs, the one whose mean is the median, noting how far the others strayed; a run
/// that failed outright outranks the rest, since the scenario did not complete every time.
fn median_run(mut results: Vec<ScenarioResult>) -> ScenarioResult {
    let runs = results.len() as u32;
    if let Some(index) = results
        .iter()
        .position(|result| result.verdict.exit_code() >= Verdict::Harness.exit_code())
    {
        let mut failed = results.swap_remove(index);
        failed.runs = runs;
        return failed;
    }
    let means: Vec<f64> = results
        .iter()
        .map(|result| result.stats.as_ref().map_or(0.0, |stats| stats.mean.0))
        .collect();
    let mut order: Vec<usize> = (0..results.len()).collect();
    order.sort_by(|&a, &b| means[a].total_cmp(&means[b]));
    let median = order[order.len() / 2];
    let spread = means
        .iter()
        .map(|mean| ((mean - means[median]) / means[median].max(f64::EPSILON)).abs())
        .fold(0.0, f64::max);
    let mut kept = results.swap_remove(median);
    kept.runs = runs;
    kept.mean_spread = (runs > 1).then_some(spread);
    kept
}

fn print_result(result: &ScenarioResult) {
    let stats = result
        .stats
        .as_ref()
        .map_or_else(|| "no frames".to_owned(), |stats| stats.summary());
    let headroom = match (result.headroom, result.headroom_met) {
        (Some(wanted), Some(false)) => format!("  (expected {:.0} fps of headroom)", wanted.0),
        _ => String::new(),
    };
    println!(
        "[{}-bench] {:<8} {:<24} {:<8} {stats}{headroom}",
        match result.target {
            Target::Client => "client",
            Target::Editor => "editor",
        },
        result.video.name(),
        result.name,
        format!("{:?}", result.verdict).to_lowercase()
    );
    for failure in &result.failures {
        println!("[bench]   {failure}");
    }
}

fn failure_text(failure: &Failure) -> String {
    match failure {
        Failure::Harness(why) => format!("harness: {why}"),
        Failure::Crashed(why) => format!("crashed: {why}"),
    }
}

/// The highest count of the game's spawn area, each with its players, the server sustains within
/// the tick budget.
fn server(content: &Content, layout: sim::Layout) {
    println!(
        "[bench] finding the highest A sustained within the {:.1}ms budget ({} players)...",
        budget_ms(),
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
        "[bench] capacity: {} isolated areas = {} NPCs + {} players sustained at {:.1}ms/tick (budget {:.1}ms)",
        areas,
        npcs,
        players,
        r.full.mean,
        budget_ms(),
    );
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
    let sim = measure_ticks(&mut bench).mean;
    bench.connect();
    Point {
        full: measure_ticks(&mut bench),
        sim,
    }
}

fn measure_ticks(bench: &mut Bench) -> Timings {
    sim::warm_up(bench, step);
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
    bench.tick(None, clock);
    bench.send_out();
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

/// The JSON Schema of the report a bench writes.
pub fn schema() -> String {
    report::schema()
}
