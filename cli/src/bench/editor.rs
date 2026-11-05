//! The editor's frame rate on every area of the game, at the lowest and the highest video
//! settings, with the whole scene tree hovered so that every module is outlined and named: how
//! long the game and the area take to open, then the area seen whole as the editor first frames
//! it, zoomed in on its middle with the mouse wheel notch by notch, and seen from as near as the
//! editor's camera starts, then flicked out to the whole and back in with as many notches at once,
//! halfway first and then the rest of the way. Every frame counts, the slowest most, and so does
//! how long the area takes to stand close up once the steady zoom stops, and halfway and close up
//! from the moment each flick in begins.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use editor::app::{FrameRecord, SceneryState};
use editor::scenes::SceneStep;
use editor::user::{EditorSettings, UserFolder};
use editor::views::area::camera::OrbitCamera;
use engine::core::content::Content;
use engine::core::units::{Bytes, Meters};
use engine::systems::area::AreaDef;
use engine::systems::probe::FrameSample;

use super::desktop::BenchDesktop;
use super::frames::{Class, Criteria, Window};
use super::report::{ScenarioResult, Verdict};
use super::scenario::Target;
use crate::binaries::{self, EDITOR};
use crate::render::VideoPreset;

/// Every area of the game at each of `videos`, each view of it a scenario of the report, of those
/// whose names match `only`.
pub fn run(
    game: &Path,
    content: &Content,
    videos: &[VideoPreset],
    only: Option<&str>,
) -> Vec<ScenarioResult> {
    let areas: Vec<String> = content
        .ids::<AreaDef>()
        .map(|area| area.path(content).to_owned())
        .collect();
    let mut results = Vec::new();
    for &video in videos {
        for area in &areas {
            let stem = Path::new(area)
                .file_stem()
                .map_or_else(|| area.clone(), |stem| stem.to_string_lossy().into_owned());
            let names: Vec<String> = VIEWS.iter().map(|view| format!("{stem} {view}")).collect();
            if only
                .is_some_and(|only| !names.iter().any(|name| crate::compare::matches(only, name)))
            {
                continue;
            }
            let run = watch(game, area, video);
            println!(
                "[editor-bench] {:<8} {:<40} opened in {:.1} s, stood close in {:.1} s, flicked halfway in {:.1} s, flicked in {:.1} s",
                video.name(),
                area,
                run.opened.as_secs_f64(),
                run.closed_in.as_secs_f64(),
                run.halfway_in.as_secs_f64(),
                run.flicked_in.as_secs_f64()
            );
            for ((view, window), name) in run.views().into_iter().zip(names) {
                if only.is_some_and(|only| !crate::compare::matches(only, &name)) {
                    continue;
                }
                let class = if view == "open" {
                    Class::Busy
                } else {
                    Class::Typical
                };
                let criteria = Criteria::global(video);
                let stats = window.stats(&criteria);
                let failures = stats.as_ref().map_or_else(
                    || vec!["no frames were logged".to_owned()],
                    |stats| criteria.failures(stats),
                );
                let headroom = class.headroom(video);
                let result = ScenarioResult {
                    name,
                    target: Target::Editor,
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
                    time_to_stand_frames: Vec::new(),
                    runs: 1,
                    mean_spread: None,
                };
                println!(
                    "[editor-bench] {:<8} {:<40} {view:<6} {}",
                    video.name(),
                    area,
                    result
                        .stats
                        .as_ref()
                        .map_or_else(|| "no frames".to_owned(), |stats| stats.summary())
                );
                results.push(result);
            }
        }
    }
    results
}

const VIEWS: [&str; 4] = ["open", "whole", "zoom", "close"];

/// What the editor did with one area: how long until it first stood whole and the frames until
/// then, the frames seen whole, zooming in and close up, and how long it took to stand close up
/// once the steady zoom stopped, and halfway and close up once flicked in.
struct AreaRun {
    opened: Duration,
    closed_in: Duration,
    halfway_in: Duration,
    flicked_in: Duration,
    open: Window,
    whole: Window,
    zoom: Window,
    close: Window,
}

impl AreaRun {
    fn views(&self) -> [(&'static str, &Window); 4] {
        [
            ("open", &self.open),
            ("whole", &self.whole),
            ("zoom", &self.zoom),
            ("close", &self.close),
        ]
    }
}

/// The editor opened on `game` with `area` in view, shown as `video` asks.
fn watch(game: &Path, area: &str, video: VideoPreset) -> AreaRun {
    let folder = tempfile::tempdir().expect("a folder for the editor's settings and log");
    let desktop = BenchDesktop::open(folder.path());
    let user = folder.path().join("user");
    UserFolder::at(user.clone()).save_settings(&EditorSettings {
        video: video.preferences(),
        ..EditorSettings::default()
    });
    let log = Log(folder.path().join("frames.jsonl"));
    let output = File::create(folder.path().join(EDITOR_LOG)).expect("an editor log");
    let since = Instant::now();
    let mut editor = Running(
        Command::new(binaries::beside(EDITOR))
            .arg(game)
            .arg("--scene")
            .arg(
                serde_json::to_string(&[
                    SceneStep::Area(area.to_owned()),
                    SceneStep::Hover { row: String::new() },
                ])
                .expect("a serializable scene"),
            )
            .arg("--user-folder")
            .arg(&user)
            .arg("--frame-log")
            .arg(&log.0)
            .envs(desktop.env())
            .stdout(
                output
                    .try_clone()
                    .expect("a second handle on the editor log"),
            )
            .stderr(output)
            .spawn()
            .expect("the editor runs"),
    );
    let pid = editor.0.id();
    desktop.seat(pid);
    let output = folder.path().join(EDITOR_LOG);
    let mut stood = |what: &str| {
        let deadline = Instant::now() + STAND_WITHIN;
        while !log
            .records()
            .last()
            .is_some_and(|record| record.scenery == SceneryState::Stood)
        {
            let exited = editor.0.try_wait().ok().flatten();
            assert!(
                exited.is_none() && Instant::now() < deadline,
                "the editor never stood {area} {what}: see {}",
                output.display()
            );
            std::thread::sleep(POLL);
        }
    };
    stood("whole");
    let opened = since.elapsed();
    let open = log.frames(0);
    let whole = log.watch(MEASURE);
    let first = log.records().len();
    let close = OrbitCamera::default().distance;
    let deadline = Instant::now() + ZOOM_WITHIN;
    let mut nearest = f32::INFINITY;
    let mut still_since = Instant::now();
    let mut notches = 0;
    while Instant::now() < deadline {
        desktop.scroll(pid, 1);
        notches += 1;
        std::thread::sleep(NOTCH);
        let Some(Meters(distance)) = log.records().last().and_then(|record| record.camera) else {
            continue;
        };
        if distance <= close.0 {
            break;
        }
        if distance < nearest {
            nearest = distance;
            still_since = Instant::now();
        } else if still_since.elapsed() > STILL {
            break;
        }
    }
    let zoom = log.frames(first);
    let zoomed = Instant::now();
    stood("close");
    let closed_in = zoomed.elapsed();
    let close = log.watch(MEASURE);
    let camera = || log.records().last().and_then(|record| record.camera);
    let near = camera();
    desktop.flick(pid, -notches);
    come_to_rest(&log, near);
    stood("flicked out");
    let far = camera();
    let half = (notches + 1) / 2;
    let flicked = Instant::now();
    desktop.flick(pid, half);
    come_to_rest(&log, far);
    stood("flicked halfway in");
    let halfway_in = flicked.elapsed();
    let flicked_in = match notches - half {
        0 => halfway_in,
        rest => {
            let halfway = camera();
            let flicked = Instant::now();
            desktop.flick(pid, rest);
            come_to_rest(&log, halfway);
            stood("flicked in");
            flicked.elapsed()
        }
    };
    AreaRun {
        opened,
        closed_in,
        halfway_in,
        flicked_in,
        open,
        whole,
        zoom,
        close,
    }
}

/// Waits for the camera to leave `from` and come to rest, as it does after the wheel turns.
fn come_to_rest(log: &Log, from: Option<Meters>) {
    let camera = || log.records().last().and_then(|record| record.camera);
    let deadline = Instant::now() + STAND_WITHIN;
    while camera() == from {
        assert!(Instant::now() < deadline, "the camera never moved");
        std::thread::sleep(POLL);
    }
    let mut last = camera();
    loop {
        std::thread::sleep(POLL);
        let now = camera();
        if now == last {
            return;
        }
        assert!(Instant::now() < deadline, "the camera never came to rest");
        last = now;
    }
}

/// The editor's frame log.
struct Log(PathBuf);

impl Log {
    /// The frames logged so far, but for a line still being written.
    fn records(&self) -> Vec<FrameRecord> {
        std::fs::read_to_string(&self.0)
            .unwrap_or_default()
            .lines()
            .map_while(|line| serde_json::from_str(line).ok())
            .collect()
    }

    /// The frames logged from the `first` on, as the editor's whole frame on its main thread.
    fn frames(&self, first: usize) -> Window {
        let mut window = Window::default();
        for (index, record) in self.records().into_iter().enumerate().skip(first) {
            window.push(FrameSample {
                frame: index as u32,
                dt_ms: record.frame,
                cpu_main_ms: record.frame,
                cpu_render_ms: None,
                gpu_ms: None,
                missing: 0,
                draws: 0,
                instances: 0,
                entities: record.entities as u32,
                visible_chunks: 0,
                upload_bytes: Bytes(0),
            });
        }
        window
    }

    /// The frames logged while the editor is left to draw for `lasting`.
    fn watch(&self, lasting: Duration) -> Window {
        let first = self.records().len();
        std::thread::sleep(lasting);
        self.frames(first)
    }
}

const EDITOR_LOG: &str = "editor.log";
const STAND_WITHIN: Duration = Duration::from_secs(180);
const MEASURE: Duration = Duration::from_secs(4);
const POLL: Duration = Duration::from_millis(100);
/// How long the wheel rests between notches, as a hand turning it steadily would.
const NOTCH: Duration = Duration::from_millis(120);
const ZOOM_WITHIN: Duration = Duration::from_secs(60);
/// How long the camera may stand still under the turning wheel before the zoom counts as done.
const STILL: Duration = Duration::from_secs(2);

struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
