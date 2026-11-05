mod desktop;
mod driver;
mod editor;
mod frames;
mod hands;
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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use engine::core::content::Content;
use engine::core::math::Size;
use engine::core::units::WindowPx;
use engine::systems::GameSettings;
use engine::systems::area::{self, AreaModule};

use crate::loadtest::Pool;
use crate::render::{VideoFilter, VideoPreset};
use crate::search::{Search, budget_ms, verdict};
use driver::Driver;
use frames::{Class, Criteria, Fps, Window};
use hands::Failure;
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
    /// The game's scenarios file, `bench/scenarios.json` in its folder unless given.
    pub scenarios: Option<PathBuf>,
    /// The video presets to run at.
    pub video: VideoFilter,
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
    let videos = args.video.presets();
    let mut report: Option<BenchReport> = None;
    if runs(BenchPart::Client) {
        client(game, &content, args, &videos, &mut report)?;
    }
    if runs(BenchPart::Editor) {
        let steps = args.report.with_extension(STEPS_LOG);
        editor::run(game, &content, args, &videos, &steps, &mut report)?;
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
    videos: &[VideoPreset],
    report: &mut Option<BenchReport>,
) -> Result<(), String> {
    let slots_file = args
        .scenarios
        .clone()
        .unwrap_or_else(|| game.join(scenarios::SLOTS_FILE));
    let slots = ScenarioSlots::of(&slots_file, content)?;
    let scenarios: Vec<Scenario> = scenarios::client::catalogue(&slots, content, args.smoke)
        .into_iter()
        .filter(|scenario| {
            args.scenario
                .as_deref()
                .is_none_or(|only| crate::compare::matches(only, &scenario.name))
        })
        .collect();
    let desktop = !args.smoke
        && args
            .scenario
            .as_deref()
            .is_none_or(|only| crate::compare::matches(only, DESKTOP));
    if scenarios.is_empty() && !desktop {
        println!("[client-bench] no scenario to run");
        return Ok(());
    }
    let steps = args.report.with_extension(STEPS_LOG);
    let _ = std::fs::remove_file(&steps);
    warn_of_load();
    if desktop {
        desktop_check(game, args, &steps, report)?;
    }
    if scenarios.is_empty() {
        return Ok(());
    }
    for &video in videos {
        let mut driver = Driver::start(game, content, slots.character, video, &steps)
            .map_err(|failure| failure_text(&failure))?;
        let report = client_report(report, &mut driver, game, args)?;
        for scenario in &scenarios {
            let result = measure(
                &mut driver,
                scenario,
                video,
                args,
                &Restart {
                    game,
                    content,
                    slots: &slots,
                    steps: &steps,
                },
            );
            print_result(&result);
            report.add(result);
            if driver_dead(&mut driver) {
                driver = Driver::start(game, content, slots.character, video, &steps)
                    .map_err(|failure| failure_text(&failure))?;
            }
        }
    }
    Ok(())
}

/// Whether the bench's desktop lets the client draw as fast as it can: the emptiest view of the
/// engine's own new game, which draws far faster than a desktop refreshes whatever the game
/// benched costs to draw, played at the lowest settings. A mean under [`DESKTOP_FLOOR`] fails the
/// harness, as the desktop then holds the frames and every scenario measures it, not the engine.
fn desktop_check(
    game: &Path,
    args: &BenchArgs,
    steps: &Path,
    report: &mut Option<BenchReport>,
) -> Result<(), String> {
    let folder = tempfile::tempdir().expect("a folder for the desktop check's game");
    let checked = folder.path().join("game");
    ::editor::templates::new_game(
        DESKTOP_TEMPLATE,
        DESKTOP,
        &checked,
        &::editor::busy::Progress::default(),
    )?;
    let content = load(&checked);
    let slots = ScenarioSlots::of(&checked.join(scenarios::SLOTS_FILE), &content)?;
    let home = slots
        .home()
        .ok_or("the engine's new game has nowhere to stand")?;
    let scenario = Scenario {
        name: DESKTOP.to_owned(),
        ..scenarios::client::empty_view(&slots, home)
    };
    let video = VideoPreset::Lowest;
    let mut driver = Driver::start(&checked, &content, slots.character, video, steps)
        .map_err(|failure| failure_text(&failure))?;
    let report = client_report(report, &mut driver, game, args)?;
    let restart = Restart {
        game: &checked,
        content: &content,
        slots: &slots,
        steps,
    };
    let mut result = measure(&mut driver, &scenario, video, args, &restart);
    if let Some(stats) = &result.stats
        && stats.mean.0 < DESKTOP_FLOOR.0
    {
        result.verdict = Verdict::Harness;
        result.failures.push(format!(
            "harness: the desktop lets the client draw the engine's emptiest view at only {:.0} fps ({:.0} expected): its compositor holds the frames",
            stats.mean.0, DESKTOP_FLOOR.0
        ));
    }
    print_result(&result);
    report.add(result);
    Ok(())
}

/// The name the desktop check is reported by.
const DESKTOP: &str = "desktop";
/// The engine's new game the desktop check draws.
const DESKTOP_TEMPLATE: &str = "new_game";
/// What the engine's emptiest view must exceed at the lowest settings for the desktop to count
/// as letting the client draw as fast as it can.
const DESKTOP_FLOOR: Fps = Fps(300.0);

/// The report, opened on the adapter and screen the first app the bench drives draws on when it
/// isn't yet; refused when an app draws on another adapter than the bench was asked for.
fn report_of<'a>(
    report: &'a mut Option<BenchReport>,
    adapter: String,
    screen: Size<WindowPx>,
    game: &Path,
    args: &BenchArgs,
) -> Result<&'a mut BenchReport, String> {
    if let Some(wanted) = &args.adapter
        && wanted != &adapter
    {
        return Err(format!(
            "the bench draws on {adapter}, not the {wanted} it was asked for"
        ));
    }
    Ok(report.get_or_insert_with(|| {
        BenchReport::new(
            adapter,
            Resolution {
                width: screen.width as u32,
                height: screen.height as u32,
            },
            game,
        )
    }))
}

/// The report, opened on the adapter and screen `driver`'s client draws on.
fn client_report<'a>(
    report: &'a mut Option<BenchReport>,
    driver: &mut Driver,
    game: &Path,
    args: &BenchArgs,
) -> Result<&'a mut BenchReport, String> {
    let adapter = driver
        .adapter()
        .map_err(|failure| failure_text(&failure))?
        .unwrap_or_default();
    let screen = driver.screen().map_err(|failure| failure_text(&failure))?;
    report_of(report, adapter, screen, game, args)
}

fn driver_dead(driver: &mut Driver) -> bool {
    driver.adapter().is_err()
}

/// A loaded machine measures the load, not the engine: said once, and kept in the report.
fn warn_of_load() {
    let Ok(load) = sysinfo::System::load_average() else {
        return;
    };
    let cores = std::thread::available_parallelism().map_or(1, |cores| cores.get());
    if load.one > cores as f64 * BUSY_SHARE {
        println!(
            "[bench] WARNING the machine's load average is {:.1} over {cores} cores: the frames measure its load too",
            load.one
        );
    }
}

/// The share of the cores a load average may reach before the machine counts as busy.
const BUSY_SHARE: f64 = 0.5;

/// Other players held on the server while a scenario plays, each answering it from a thread of
/// their own, gone when dropped.
struct Crowd {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Crowd {
    /// `count` players joined to the server at `http_url`; refused when fewer got in.
    fn join(http_url: String, count: u32) -> Result<Crowd, Failure> {
        let stop = Arc::new(AtomicBool::new(false));
        let (joined, report) = mpsc::channel();
        let thread = std::thread::spawn({
            let stop = Arc::clone(&stop);
            move || {
                let mut pool = Pool::open(&http_url, count as usize);
                while !stop.load(Ordering::Relaxed) {
                    let _ = joined.send(pool.connected());
                    pool.hold(CROWD_SLICE);
                }
            }
        });
        let crowd = Crowd {
            stop,
            thread: Some(thread),
        };
        let since = Instant::now();
        let mut connected = 0;
        while connected < count as usize {
            connected = report
                .recv_timeout(JOIN_WITHIN)
                .map_err(|_| Failure::Harness("the players never joined".to_owned()))?;
            if since.elapsed() > JOIN_WITHIN {
                return Err(Failure::Harness(format!(
                    "only {connected} of {count} players joined the server within {} s",
                    JOIN_WITHIN.as_secs()
                )));
            }
        }
        Ok(crowd)
    }
}

impl Drop for Crowd {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

const CROWD_SLICE: Duration = Duration::from_millis(250);
/// How long the players have to join before the scenario gives up on them.
const JOIN_WITHIN: Duration = Duration::from_secs(60);

/// What starting a client over takes, when one dies mid-scenario.
struct Restart<'a> {
    game: &'a Path,
    content: &'a Content,
    slots: &'a ScenarioSlots,
    steps: &'a Path,
}

/// The extension the log of every step the client was put through takes, beside the report.
const STEPS_LOG: &str = "steps.log";

/// The scenario run `repeat` times, the median run by its mean kept, with how far the others
/// strayed from it.
fn measure(
    driver: &mut Driver,
    scenario: &Scenario,
    video: VideoPreset,
    args: &BenchArgs,
    restart: &Restart,
) -> ScenarioResult {
    let criteria = scenario.criteria.map_or(Criteria::global(video), |own| {
        Criteria::global(video).stricter(own)
    });
    let mut results = Vec::new();
    for run in 0..args.repeat.max(1) {
        let started = Instant::now();
        let outcome = match scenario.players {
            0 => driver.run(scenario),
            players => Crowd::join(driver.server_url(), players).and_then(|crowd| {
                let measured = driver.run(scenario);
                drop(crowd);
                measured
            }),
        };
        let result = result_of(
            &scenario.name,
            scenario.target,
            scenario.class,
            video,
            &criteria,
            outcome.map(|measured| (measured.window, measured.stood_after_frames)),
            args.smoke,
        );
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
            match Driver::start(
                restart.game,
                restart.content,
                restart.slots.character,
                video,
                restart.steps,
            ) {
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

/// What a run of a scenario came to: its frames measured against `criteria` with how long its
/// area took to stand after each move, or why it could not be played through.
fn result_of(
    name: &str,
    target: Target,
    class: Class,
    video: VideoPreset,
    criteria: &Criteria,
    outcome: Result<(Window, Vec<u32>), Failure>,
    smoke: bool,
) -> ScenarioResult {
    let headroom = class.headroom(video);
    match outcome {
        Ok((window, stood_after_frames)) => {
            let stats = window.stats(criteria);
            let failures = match (&stats, smoke) {
                (Some(stats), false) => criteria.failures(stats),
                (None, _) => vec!["no frames were measured".to_owned()],
                (Some(_), true) => Vec::new(),
            };
            ScenarioResult {
                name: name.to_owned(),
                target,
                class,
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
                time_to_stand_frames: stood_after_frames,
                runs: 1,
                mean_spread: None,
            }
        }
        Err(failure) => ScenarioResult {
            name: name.to_owned(),
            target,
            class,
            video,
            verdict: match failure {
                Failure::Harness(_) => Verdict::Harness,
                Failure::Crashed(_) => Verdict::Crashed,
            },
            failures: vec![failure_text(&failure)],
            stats: None,
            headroom,
            headroom_met: None,
            time_to_stand_frames: Vec::new(),
            runs: 1,
            mean_spread: None,
        },
    }
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
