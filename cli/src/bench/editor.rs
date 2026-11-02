//! The editor's frame rate on every area of the game, seen whole as the editor first frames it and
//! close up from the editor's own camera distance, so a view over a whole area is held to cost
//! about what a close one does.

use std::fs::File;
use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use editor::app::{FrameRecord, SceneryState};
use editor::scenes::SceneStep;
use editor::views::area::camera::OrbitCamera;
use engine::core::content::Content;
use engine::systems::area::AreaDef;

use super::desktop::BenchDesktop;
use crate::binaries::{self, EDITOR};

pub fn run(game: &Path, content: &Content) {
    let views: Vec<View> = content
        .ids::<AreaDef>()
        .map(|area| area.path(content).to_owned())
        .flat_map(|area| {
            [Framing::Whole, Framing::Close].map(|framing| watch(game, &area, framing))
        })
        .collect();
    report(&views);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Framing {
    /// The whole area in view, as the editor first frames it.
    Whole,
    /// The middle of the area from as near as the editor's camera starts.
    Close,
}

impl Framing {
    fn name(self) -> &'static str {
        match self {
            Framing::Whole => "whole",
            Framing::Close => "close",
        }
    }
}

struct View {
    area: String,
    framing: Framing,
    frames: usize,
    lasted: Duration,
}

impl View {
    fn fps(&self) -> f64 {
        self.frames as f64 / self.lasted.as_secs_f64()
    }
}

/// The editor opened on `area` framed as asked, watched once all it takes in stands.
fn watch(game: &Path, area: &str, framing: Framing) -> View {
    let folder = tempfile::tempdir().expect("a folder for the editor's settings and log");
    let desktop = BenchDesktop::open(folder.path());
    let mut scene = vec![SceneStep::Area(area.to_owned())];
    if framing == Framing::Close {
        scene.push(SceneStep::Camera {
            distance: OrbitCamera::default().distance,
        });
    }
    let frames = folder.path().join("frames.jsonl");
    let log = File::create(folder.path().join(EDITOR_LOG)).expect("an editor log");
    let editor = Running(
        Command::new(binaries::beside(EDITOR))
            .arg(game)
            .arg("--scene")
            .arg(serde_json::to_string(&scene).expect("a serializable scene"))
            .arg("--user-folder")
            .arg(folder.path().join("user"))
            .arg("--frame-log")
            .arg(&frames)
            .envs(desktop.env())
            .stdout(log.try_clone().expect("a second handle on the editor log"))
            .stderr(log)
            .spawn()
            .expect("the editor runs"),
    );
    desktop.seat(editor.0.id());
    let since = Instant::now();
    while !records(&frames)
        .last()
        .is_some_and(|record| record.scenery == SceneryState::Stood)
    {
        assert!(
            since.elapsed() < STAND_WITHIN,
            "the editor never stood {area} {}: see {}",
            framing.name(),
            folder.path().join(EDITOR_LOG).display()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    let first = records(&frames).len();
    let since = Instant::now();
    std::thread::sleep(MEASURE);
    let last = records(&frames).len();
    View {
        area: area.to_owned(),
        framing,
        frames: last - first,
        lasted: since.elapsed(),
    }
}

/// The frames the editor has logged so far, but for a line it is still writing.
fn records(log: &Path) -> Vec<FrameRecord> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map_while(|line| serde_json::from_str(line).ok())
        .collect()
}

fn report(views: &[View]) {
    for view in views {
        println!(
            "[editor-bench] {:<40} {} {:>6.0} fps",
            view.area,
            view.framing.name(),
            view.fps()
        );
    }
    let lowest = |framing: Framing| {
        views
            .iter()
            .filter(|view| view.framing == framing)
            .min_by(|a, b| a.fps().total_cmp(&b.fps()))
    };
    let (Some(whole), Some(close)) = (lowest(Framing::Whole), lowest(Framing::Close)) else {
        println!("[editor-bench] not benched: the game has no area");
        return;
    };
    println!(
        "[editor-bench] RESULT whole={:.0} ({}) close={:.0} ({})",
        whole.fps(),
        whole.area,
        close.fps(),
        close.area
    );
}

const EDITOR_LOG: &str = "editor.log";
const STAND_WITHIN: Duration = Duration::from_secs(120);
const MEASURE: Duration = Duration::from_secs(4);

struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
