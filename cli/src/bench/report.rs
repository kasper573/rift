//! What a bench run reports: the build and the machine it ran on, the server's ticks at its
//! load, and every scenario's frames against its criteria, written as `bench-report.json` for
//! `rift schema bench` to describe and for a later run to compare against.

use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use engine::core::units::Millis;

use super::frames::{Class, Fps, FrameStats};
use super::scenario::Target;
use crate::render::VideoPreset;

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct BenchReport {
    pub engine_build: String,
    /// The revision of the game's repository, where it has one.
    pub game_revision: Option<String>,
    pub machine: Machine,
    /// The machine's load average over the minute before the run, as its kernel reports it:
    /// a loaded machine measures nothing of the engine.
    pub load_average: f64,
    /// The server's ticks at the load it was measured at, where the server part ran at one.
    pub server: Option<ServerRun>,
    /// What the client and the editor drew on, where either ran.
    pub screen: Option<Screen>,
    pub scenarios: Vec<ScenarioResult>,
    pub overall: Verdict,
}

impl BenchReport {
    /// A report of a run on `game`, or on the server bench's own game.
    pub fn new(game: Option<&Path>) -> BenchReport {
        BenchReport {
            engine_build: engine::core::content::version::build().to_owned(),
            game_revision: game.and_then(revision_of),
            machine: Machine::this(),
            load_average: sysinfo::System::load_average().map_or(0.0, |load| load.one),
            server: None,
            screen: None,
            scenarios: Vec::new(),
            overall: Verdict::Passed,
        }
    }

    pub fn add(&mut self, result: ScenarioResult) {
        if result.verdict.exit_code() > self.overall.exit_code() {
            self.overall = result.verdict;
        }
        self.scenarios.push(result);
    }

    pub fn write(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self).map_err(|error| error.to_string())?;
        std::fs::write(path, json).map_err(|error| format!("{}: {error}", path.display()))
    }

    pub fn read(path: &Path) -> Result<BenchReport, String> {
        let json = std::fs::read_to_string(path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        serde_json::from_str(&json).map_err(|error| format!("{}: {error}", path.display()))
    }

    /// Every scenario of this run beside the same scenario of `before`, and what got worse: a
    /// scenario that passed and now fails, a mean that fell by more than a tenth, or a p99 that
    /// rose by more than a fifth.
    pub fn compare(&self, before: &BenchReport) -> Comparison {
        let mut rows = Vec::new();
        let mut worse = Vec::new();
        if let (Some(was), Some(now)) = (&before.server, &self.server) {
            compare_servers(was, now, &mut rows, &mut worse);
        }
        for now in &self.scenarios {
            let was = before
                .scenarios
                .iter()
                .find(|was| was.name == now.name && was.video == now.video);
            let Some(was) = was else {
                rows.push(format!("{:<40} {:<8} new", now.name, now.video.name()));
                continue;
            };
            let key = format!("{} at {}", now.name, now.video.name());
            if was.verdict == Verdict::Passed && now.verdict != Verdict::Passed {
                worse.push(format!("{key}: passed before, now {:?}", now.verdict));
            }
            match (&was.stats, &now.stats) {
                (Some(was), Some(now_stats)) => {
                    let mean = change(was.mean.0, now_stats.mean.0);
                    let p99 = change(f64::from(was.p99.0), f64::from(now_stats.p99.0));
                    rows.push(format!(
                        "{:<40} {:<8} mean {:>5.0} → {:>5.0} fps ({:+.0}%)  p99 {:>6.1} → {:>6.1} ms ({:+.0}%)",
                        now.name,
                        now.video.name(),
                        was.mean.0,
                        now_stats.mean.0,
                        mean * 100.0,
                        was.p99.0,
                        now_stats.p99.0,
                        p99 * 100.0
                    ));
                    if mean < -MEAN_TOLERANCE {
                        worse.push(format!(
                            "{key}: the mean fell from {:.0} to {:.0} fps",
                            was.mean.0, now_stats.mean.0
                        ));
                    }
                    if p99 > P99_TOLERANCE {
                        worse.push(format!(
                            "{key}: p99 rose from {:.1} to {:.1} ms",
                            was.p99.0, now_stats.p99.0
                        ));
                    }
                }
                _ => rows.push(format!(
                    "{:<40} {:<8} {:?} → {:?}",
                    now.name,
                    now.video.name(),
                    was.verdict,
                    now.verdict
                )),
            }
        }
        Comparison { rows, worse }
    }
}

/// How much worse a scenario, or the server's tick, may measure than before without failing the
/// comparison.
const MEAN_TOLERANCE: f64 = 0.10;
const P99_TOLERANCE: f64 = 0.20;

/// The server's ticks now beside before, and whether they got worse: a mean tick longer by more
/// than a tenth, or a p99 by more than a fifth. A run at another load is told, not compared.
fn compare_servers(
    was: &ServerRun,
    now: &ServerRun,
    rows: &mut Vec<String>,
    worse: &mut Vec<String>,
) {
    if (was.areas, was.players, was.npcs) != (now.areas, now.players, now.npcs) {
        rows.push(format!(
            "{:<40} measured at {} areas, before at {}: not compared",
            "server", now.areas, was.areas
        ));
        return;
    }
    let mean = change(f64::from(was.tick.mean.0), f64::from(now.tick.mean.0));
    let p99 = change(f64::from(was.tick.p99.0), f64::from(now.tick.p99.0));
    rows.push(format!(
        "{:<40} {:<8} mean {:>6.2} → {:>6.2} ms ({:+.0}%)  p99 {:>6.2} → {:>6.2} ms ({:+.0}%)",
        "server",
        format!("{}", now.areas),
        was.tick.mean.0,
        now.tick.mean.0,
        mean * 100.0,
        was.tick.p99.0,
        now.tick.p99.0,
        p99 * 100.0
    ));
    if mean > MEAN_TOLERANCE {
        worse.push(format!(
            "server: the mean tick rose from {:.2} to {:.2} ms",
            was.tick.mean.0, now.tick.mean.0
        ));
    }
    if p99 > P99_TOLERANCE {
        worse.push(format!(
            "server: p99 rose from {:.2} to {:.2} ms",
            was.tick.p99.0, now.tick.p99.0
        ));
    }
}

fn change(was: f64, now: f64) -> f64 {
    if was.abs() < f64::EPSILON {
        0.0
    } else {
        (now - was) / was
    }
}

pub struct Comparison {
    pub rows: Vec<String>,
    pub worse: Vec<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct Machine {
    pub os: String,
    pub cpu: String,
    pub cores: usize,
    pub memory_gb: f64,
}

impl Machine {
    fn this() -> Machine {
        use sysinfo::System;
        let mut system = System::new().expect("the system to read");
        system.refresh_cpu_list(sysinfo::CpuRefreshKind::nothing());
        system.refresh_memory();
        Machine {
            os: System::long_os_version().unwrap_or_else(|_| std::env::consts::OS.to_owned()),
            cpu: system
                .cpus()
                .first()
                .map(|cpu| cpu.brand().trim().to_owned())
                .unwrap_or_default(),
            cores: system.cpus().len(),
            memory_gb: system.total_memory() as f64 / (1024.0 * 1024.0 * 1024.0),
        }
    }
}

/// The server's ticks at one load, the median of the runs measured.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ServerRun {
    /// The copies of the game's spawn area the server ticked, each a world of its own.
    pub areas: usize,
    pub players: usize,
    pub npcs: usize,
    /// How many of the players played each role.
    pub roles: Vec<RoleCount>,
    /// A whole tick, the players' clients sent what changed.
    pub tick: TickStats,
    /// The mean tick before the clients connected: the simulation alone.
    pub simulation: Millis,
    /// What the clients add to the mean tick: replicating the world to them.
    pub replication: Millis,
    pub runs: u32,
    /// Over the runs, how far apart the slowest and fastest mean ticks lay.
    pub spread: Millis,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct RoleCount {
    pub role: String,
    pub players: usize,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug)]
pub struct TickStats {
    pub mean: Millis,
    pub p50: Millis,
    pub p99: Millis,
    pub max: Millis,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct Screen {
    pub adapter: String,
    pub resolution: Resolution,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resolution {
    pub width: u32,
    pub height: u32,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ScenarioResult {
    pub name: String,
    pub target: Target,
    pub class: Class,
    pub video: VideoPreset,
    pub verdict: Verdict,
    /// Why it failed, each reason in a line.
    pub failures: Vec<String>,
    pub stats: Option<FrameStats>,
    /// The frame rate the scenario's class expects over the floor, and whether it was met.
    pub headroom: Option<Fps>,
    pub headroom_met: Option<bool>,
    /// Of an arrival, how many frames the area took to stand after each move.
    pub time_to_stand_frames: Vec<u32>,
    /// How many times the scenario was run, this result the median run by its mean.
    pub runs: u32,
    /// Over the runs, how far the mean strayed from the median run's, as a share of it.
    pub mean_spread: Option<f64>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Passed,
    /// The frames fell short of the criteria.
    Criteria,
    /// The script could not be played through: an input never confirmed, a target not found, or
    /// the probe's ring of frames wrapped before it was read.
    Harness,
    /// The client or the server died.
    Crashed,
}

impl Verdict {
    pub fn exit_code(self) -> i32 {
        match self {
            Verdict::Passed => 0,
            Verdict::Criteria => 1,
            Verdict::Harness => 2,
            Verdict::Crashed => 3,
        }
    }
}

pub fn schema() -> String {
    let schema = schemars::schema_for!(BenchReport);
    serde_json::to_string_pretty(&schema).expect("a serializable schema")
}

fn revision_of(game: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(game)
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}
