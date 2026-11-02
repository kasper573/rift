//! The editor's frame rate on every area of the game, at the lowest and the highest video
//! settings, with the whole scene tree hovered so that every module is outlined and named: how
//! long the game and the area take to open, then the area seen whole as the editor first frames
//! it, zoomed in on its middle with the mouse wheel notch by notch, and seen from as near as the
//! editor's camera starts. Every frame counts, the slowest most.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use editor::app::{FrameRecord, SceneryState};
use editor::scenes::SceneStep;
use editor::user::{EditorSettings, UserFolder};
use editor::views::area::camera::OrbitCamera;
use engine::core::content::Content;
use engine::core::units::Meters;
use engine::systems::area::AreaDef;

use super::desktop::BenchDesktop;
use super::frames::{Frames, Video};
use crate::binaries::{self, EDITOR};

pub fn run(game: &Path, content: &Content) {
    let areas: Vec<String> = content
        .ids::<AreaDef>()
        .map(|area| area.path(content).to_owned())
        .collect();
    for video in Video::ALL {
        let runs: Vec<AreaRun> = areas.iter().map(|area| watch(game, area, video)).collect();
        report(video, &runs);
    }
}

/// What the editor did with one area: how long until it first stood whole and the frames until
/// then, and the frames seen whole, zooming in and close up.
struct AreaRun {
    area: String,
    opened: Duration,
    open: Frames,
    whole: Frames,
    zoom: Frames,
    close: Frames,
}

impl AreaRun {
    fn views(&self) -> [(&'static str, &Frames); 4] {
        [
            ("open", &self.open),
            ("whole", &self.whole),
            ("zoom", &self.zoom),
            ("close", &self.close),
        ]
    }
}

/// The editor opened on `game` with `area` in view, shown as `video` asks.
fn watch(game: &Path, area: &str, video: Video) -> AreaRun {
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
    while Instant::now() < deadline {
        desktop.scroll(pid, 1);
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
    stood("close");
    let close = log.watch(MEASURE);
    AreaRun {
        area: area.to_owned(),
        opened,
        open,
        whole,
        zoom,
        close,
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

    /// The frames logged from the `first` on.
    fn frames(&self, first: usize) -> Frames {
        Frames::of(
            self.records()
                .into_iter()
                .skip(first)
                .map(|record| record.frame),
        )
    }

    /// The frames logged while the editor is left to draw for `lasting`.
    fn watch(&self, lasting: Duration) -> Frames {
        let first = self.records().len();
        std::thread::sleep(lasting);
        self.frames(first)
    }
}

fn report(video: Video, runs: &[AreaRun]) {
    for run in runs {
        println!(
            "[editor-bench] {:<8} {:<40} opened in {:.1} s",
            video.name(),
            run.area,
            run.opened.as_secs_f64()
        );
        for (view, frames) in run.views() {
            println!(
                "[editor-bench] {:<8} {:<40} {view:<6} {}",
                video.name(),
                run.area,
                frames.summary()
            );
        }
    }
    let slowest = runs
        .iter()
        .flat_map(|run| {
            run.views()
                .into_iter()
                .filter(|(view, _)| *view != "open")
                .map(move |(view, frames)| (run, view, frames))
        })
        .filter(|(_, _, frames)| !frames.is_empty())
        .min_by(|a, b| a.2.slowest().total_cmp(&b.2.slowest()));
    let Some((run, view, frames)) = slowest else {
        println!("[editor-bench] not benched: the game has no area");
        return;
    };
    let longest_open = runs
        .iter()
        .max_by_key(|run| run.opened)
        .expect("an area was opened");
    println!(
        "[editor-bench] RESULT video={} slowest={:.0} ({} {view}) floor={:.0} longest_open={:.1}s ({}) open_slowest={:.0}",
        video.name(),
        frames.slowest(),
        run.area,
        video.floor(),
        longest_open.opened.as_secs_f64(),
        longest_open.area,
        longest_open.open.slowest()
    );
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
