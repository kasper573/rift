//! What a bench run reports: the build and the machine it ran on, and every scenario's frames
//! against its criteria, written as `bench-report.json` for `rift schema bench` to describe and
//! for a later run to compare against.

use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::frames::{Class, Fps, FrameStats, Video};
use super::scenario::Target;

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct BenchReport {
    pub engine_build: String,
    /// The revision of the game's repository, where it has one.
    pub game_revision: Option<String>,
    pub machine: Machine,
    pub adapter: String,
    pub resolution: Resolution,
    pub scenarios: Vec<ScenarioResult>,
    pub overall: Verdict,
}

impl BenchReport {
    pub fn new(adapter: String, resolution: Resolution, game: &Path) -> BenchReport {
        BenchReport {
            engine_build: engine::core::content::version::build().to_owned(),
            game_revision: revision_of(game),
            machine: Machine::this(),
            adapter,
            resolution,
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

/// How much worse a scenario may measure than before without failing the comparison.
const MEAN_TOLERANCE: f64 = 0.10;
const P99_TOLERANCE: f64 = 0.20;

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
    pub video: Video,
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
