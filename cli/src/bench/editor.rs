//! Plays the editor's scenarios on the editor as a designer works it: opened on the game with an
//! area on view, every input sent through the X server and confirmed through
//! the editor's probe before the next, its targets found in what the probe reports of the trees,
//! the stage and the interface. The frames come from the editor's own clock, read from the probe's
//! ring every few seconds, so the pace of the input only paces the script.

use std::fs::File;
use std::io::Write;
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, ExitStatus};
use std::time::{Duration, Instant};

use editor::app::{DrawnPass, FrameRecord};
use editor::scenes::SceneStep;
use editor::user::{EditorSettings, UserFolder};
use engine::core::content::Content;
use engine::core::math::{Pos, Size};
use engine::core::units::WindowPx;
use engine::systems::GameSettings;
use engine::systems::input::map::KeyModifiers;
use engine::systems::launch::ProbeDriver;
use engine::systems::probe::{
    EditorFrameQuestion, EditorScenery, EditorSnapshot, EditorStage, ProbeFrames, ProbeInput,
    ProbeInputSeen,
};
use serde::de::DeserializeOwned;
use tempfile::TempDir;

use super::desktop::BenchDesktop;
use super::frames::{Class, Criteria, Window};
use super::hands::{self, BUDGET, Driven, Failure};
use super::report::BenchReport;
use super::scenario::{Scenario, Target};
use super::scenarios::ScenarioSlots;
use super::scenarios::editor::{EditorScenario, OPENING};
use super::{BenchArgs, failure_text, median_run, print_result, report_of, result_of};
use crate::binaries::{self, EDITOR};
use crate::render::VideoPreset;
use crate::server::LOCALHOST;

/// Every editor scenario whose name `args` asks for, at each of `videos`, on an editor of its own
/// per preset, started over when one dies: the overviews on the area of the home place `slots`
/// name, the rest on the area the game starts in. The opening is measured on every editor started
/// for it, as it is what starting one does.
pub fn run(
    game: &Path,
    content: &Content,
    slots: &ScenarioSlots,
    args: &BenchArgs,
    videos: &[VideoPreset],
    steps: &Path,
    report: &mut Option<BenchReport>,
) -> Result<(), String> {
    let wanted = |name: &str| {
        args.scenario
            .as_deref()
            .is_none_or(|only| super::named_by(only, name))
    };
    let scenarios: Vec<EditorScenario> = super::scenarios::editor::catalogue()
        .into_iter()
        .filter(|scenario| wanted(scenario.name))
        .collect();
    let overviews: Vec<EditorScenario> = super::scenarios::editor::overview_catalogue()
        .into_iter()
        .filter(|scenario| wanted(scenario.name))
        .collect();
    let opening = wanted(OPENING);
    if scenarios.is_empty() && overviews.is_empty() && !opening {
        println!("[editor-bench] no scenario to run");
        return Ok(());
    }
    let area = content
        .built::<GameSettings>()
        .initial_spawn
        .area
        .path(content)
        .to_owned();
    let home = slots
        .home
        .map(|place| place.area.path(content).to_owned())
        .ok_or("the scenarios have no home place")?;
    let start = |video| EditorDriver::start(game, &area, &[], video, steps);
    let start_home = |video| EditorDriver::start(game, &home, &[], video, steps);
    for &video in videos {
        let mut openings = Vec::new();
        let mut driver = None;
        for _ in 0..if opening { args.repeat.max(1) } else { 1 } {
            drop(driver.take());
            let mut started = start(video).map_err(|failure| failure_text(&failure))?;
            let opened = started.opened();
            openings.push(result_of(
                OPENING,
                Target::Editor,
                Class::Busy,
                video,
                &Criteria::opening(video),
                opened.map(|(window, stood)| (window, vec![stood])),
                args.smoke,
            ));
            driver = Some(started);
        }
        let mut driver = driver.expect("an editor started");
        let report = report_of(
            report,
            driver.adapter().map_err(|failure| failure_text(&failure))?,
            driver.screen().map_err(|failure| failure_text(&failure))?,
            game,
            args,
        )?;
        if opening {
            let result = median_run(openings);
            print_result(&result);
            report.add(result);
        }
        play(&mut driver, &scenarios, video, args, report, &start)?;
        if !overviews.is_empty() {
            driver.end();
            driver = start_home(video).map_err(|failure| failure_text(&failure))?;
            driver.opened().map_err(|failure| failure_text(&failure))?;
            play(&mut driver, &overviews, video, args, report, &start_home)?;
        }
    }
    Ok(())
}

/// Every one of `scenarios` measured on `driver` at `video`, as often as `args` repeats them, the
/// driver started anew by `start` when it dies.
fn play(
    driver: &mut EditorDriver,
    scenarios: &[EditorScenario],
    video: VideoPreset,
    args: &BenchArgs,
    report: &mut BenchReport,
    start: &dyn Fn(VideoPreset) -> Result<EditorDriver, Failure>,
) -> Result<(), String> {
    let criteria = Criteria::global(video);
    for scenario in scenarios {
        let mut runs = Vec::new();
        for run in 0..args.repeat.max(1) {
            let began = Instant::now();
            let measured = driver.measure(scenario);
            let crashed = matches!(measured, Err(Failure::Crashed(_)));
            runs.push(result_of(
                scenario.name,
                Target::Editor,
                scenario.class,
                video,
                &criteria,
                measured.map(|window| (window, Vec::new())),
                args.smoke,
            ));
            println!(
                "[editor-bench] {:<8} {:<24} run {} of {} in {:.0} s",
                video.name(),
                scenario.name,
                run + 1,
                args.repeat.max(1),
                began.elapsed().as_secs_f64()
            );
            if crashed || driver.exited().is_some() {
                driver.end();
                *driver = start(video).map_err(|failure| failure_text(&failure))?;
                driver.opened().map_err(|failure| failure_text(&failure))?;
            }
        }
        let result = median_run(runs);
        print_result(&result);
        report.add(result);
    }
    Ok(())
}

/// An editor on the game, played through its window.
pub struct EditorDriver {
    editor: Child,
    probe: ProbeDriver,
    desktop: BenchDesktop,
    folder: TempDir,
    seen: ProbeInputSeen,
    cursor: Option<Pos<WindowPx>>,
    frame: u32,
    measuring: Option<Measuring>,
    steps: File,
}

impl EditorDriver {
    /// The editor opened on `game` with `area` in view and `then` played after, shown as `video`
    /// asks, on a desktop of its own, every step it is put through noted in the log at `steps`.
    /// Its frames count from its first, for [`EditorDriver::opened`].
    pub fn start(
        game: &Path,
        area: &str,
        then: &[SceneStep],
        video: VideoPreset,
        steps: &Path,
    ) -> Result<EditorDriver, Failure> {
        let folder = tempfile::tempdir().expect("a folder for the editor's settings and log");
        let user = folder.path().join("user");
        UserFolder::at(user.clone()).save_settings(&EditorSettings {
            video: video.preferences(),
            ..EditorSettings::default()
        });
        let desktop = BenchDesktop::open(folder.path());
        let listener = TcpListener::bind((LOCALHOST, 0)).expect("a probe port");
        let log = File::create(folder.path().join(EDITOR_LOG)).expect("an editor log");
        let mut editor = Command::new(binaries::beside(EDITOR))
            .arg(game)
            .arg("--scene")
            .arg(
                serde_json::to_string(
                    &[vec![SceneStep::Area(area.to_owned())], then.to_vec()].concat(),
                )
                .expect("a serializable scene"),
            )
            .arg("--user-folder")
            .arg(&user)
            .arg("--frame-log")
            .arg(folder.path().join(FRAME_LOG))
            .arg("--probe")
            .arg(
                listener
                    .local_addr()
                    .expect("a bound probe port")
                    .to_string(),
            )
            .envs(desktop.env())
            .stdout(log.try_clone().expect("a second handle on the editor log"))
            .stderr(log)
            .spawn()
            .map_err(|error| Failure::Crashed(format!("the editor doesn't run: {error}")))?;
        let probe = match ProbeDriver::accept(&listener, &mut editor, LONG) {
            Ok(probe) => probe,
            Err(problem) => {
                let _ = editor.kill();
                let _ = editor.wait();
                return Err(Failure::Crashed(format!(
                    "{problem}; its log is {}",
                    folder.path().join(EDITOR_LOG).display()
                )));
            }
        };
        desktop.seat(editor.id());
        let steps = File::options()
            .append(true)
            .create(true)
            .open(steps)
            .map_err(|error| Failure::Harness(format!("{}: {error}", steps.display())))?;
        Ok(EditorDriver {
            editor,
            probe,
            desktop,
            folder,
            seen: ProbeInputSeen::default(),
            cursor: None,
            frame: 0,
            measuring: Some(Measuring::since(0)),
            steps,
        })
    }

    /// The frames from the editor's first until the area it opened on stands whole in view, and
    /// the frame it stood at.
    pub fn opened(&mut self) -> Result<(Window, u32), Failure> {
        self.note(&format!("=== {OPENING}"));
        let outcome = hands::wait_for(self, "the area to stand in view", LONG, |snapshot| {
            (snapshot.stage.is_some() && snapshot.scenery == EditorScenery::Stood && !snapshot.busy)
                .then_some(snapshot.frame)
        });
        let measured = self.read_frames(true);
        let measuring = self.measuring.take().expect("measuring the opening");
        let stood = outcome?;
        measured?;
        Ok((measuring.window, stood))
    }

    /// Sets `scenario` up and plays its script, measuring the script's frames, then leaves the
    /// editor as the scenario found it: its teardown played, whatever was left open dismissed and
    /// every edit it made undone.
    pub fn measure(&mut self, scenario: &EditorScenario) -> Result<Window, Failure> {
        self.note(&format!("=== {}", scenario.name));
        let before = self.snapshot()?;
        let played = (scenario.setup)(self).and_then(|()| {
            let start = self.snapshot()?.frame;
            self.measuring = Some(Measuring::since(start));
            let began = Instant::now();
            let outcome = loop {
                if let Err(failure) = (scenario.script)(self) {
                    break Err(failure);
                }
                if let Err(failure) = self.read_frames(true) {
                    break Err(failure);
                }
                if began.elapsed().as_secs_f32() >= Scenario::LEAST.0 {
                    break Ok(());
                }
            };
            let mut measuring = self.measuring.take().expect("measuring");
            measuring.window.note_passes(self.logged_passes());
            outcome.map(|()| measuring.window.after_warm_up(Scenario::WARM_UP))
        });
        let restored = self.restore(scenario, &before);
        let window = played?;
        restored?;
        Ok(window)
    }

    /// What the editor drew over each frame it has logged so far.
    fn logged_passes(&self) -> Vec<(u32, DrawnPass)> {
        let log = std::fs::read_to_string(self.folder.path().join(FRAME_LOG)).unwrap_or_default();
        log.lines()
            .filter_map(|line| serde_json::from_str::<FrameRecord>(line).ok())
            .map(|record| (record.sample.frame, record.pass))
            .collect()
    }

    pub fn adapter(&mut self) -> Result<String, Failure> {
        Ok(self.snapshot()?.adapter.unwrap_or_default())
    }

    pub fn screen(&mut self) -> Result<Size<WindowPx>, Failure> {
        self.snapshot()?
            .screen
            .ok_or_else(|| Failure::Harness("the editor reports no screen".to_owned()))
    }

    /// What the editor shows in a frame after asking, with the frames read when they are due.
    pub fn snapshot(&mut self) -> Result<EditorSnapshot, Failure> {
        self.read_frames(false)?;
        let snapshot: EditorSnapshot = self.ask(&EditorFrameQuestion::Snapshot)?;
        self.seen = snapshot.input.seen;
        self.cursor = snapshot.input.cursor;
        self.frame = snapshot.frame;
        Ok(snapshot)
    }

    /// The stage in view.
    pub fn stage(&mut self) -> Result<EditorStage, Failure> {
        self.snapshot()?
            .stage
            .ok_or_else(|| Failure::Harness("the editor shows no stage".to_owned()))
    }

    /// The stage once its camera has come to rest, as it does a little after the input moving it.
    pub fn stage_at_rest(&mut self) -> Result<EditorStage, Failure> {
        let mut last: Option<EditorStage> = None;
        let mut still = 0;
        hands::wait_for(self, "the camera to come to rest", BUDGET, |snapshot| {
            let stage = snapshot.stage.clone()?;
            let same = last.as_ref().is_some_and(|last| {
                last.yaw == stage.yaw
                    && last.pitch == stage.pitch
                    && last.distance == stage.distance
                    && last.target == stage.target
            });
            still = if same { still + 1 } else { 0 };
            last = Some(stage.clone());
            (still >= REST_SNAPSHOTS).then_some(stage)
        })
    }

    /// Presses `key` with `held` down, as a shortcut is pressed.
    pub fn shortcut(&mut self, held: KeyModifiers, key: &str) -> Result<EditorSnapshot, Failure> {
        let mut landed = None;
        hands::hold(self, &held, |driver| {
            landed = Some(hands::key(driver, key)?);
            Ok(())
        })?;
        landed.ok_or_else(|| Failure::Harness(format!("the shortcut {key} never landed")))
    }

    /// Undoes the latest edit, as Ctrl Z does.
    pub fn undo(&mut self) -> Result<(), Failure> {
        let before = self.snapshot()?.undoable;
        if before == 0 {
            return Err(Failure::Harness("there was no edit to undo".to_owned()));
        }
        self.shortcut(CTRL, "z")?;
        hands::wait_for(self, "the edit to be undone", BUDGET, |snapshot| {
            (snapshot.undoable < before).then_some(())
        })
    }

    /// Whether an edit came of the input just sent and held, once the editor has drawn a few frames
    /// after it and checked the folder loads with it: `before` edits stood undoable before the
    /// input.
    pub fn edited_since(&mut self, before: usize) -> Result<bool, Failure> {
        hands::frames(self, EDIT_SETTLE)?;
        let undoable = hands::wait_for(self, "the editor to check the edit", BUDGET, |snapshot| {
            (!snapshot.checking).then_some(snapshot.undoable)
        })?;
        Ok(undoable > before)
    }

    /// Ends the editor, as the driver ends it once dropped: before another starts, so the two
    /// never draw at once.
    pub fn end(&mut self) {
        let _ = self.editor.kill();
        let _ = self.editor.wait();
    }

    /// How the editor exited, once it has.
    pub fn exited(&mut self) -> Option<ExitStatus> {
        self.editor.try_wait().ok().flatten()
    }

    fn restore(
        &mut self,
        scenario: &EditorScenario,
        before: &EditorSnapshot,
    ) -> Result<(), Failure> {
        (scenario.teardown)(self)?;
        let mut dismissed = 0;
        while self.snapshot()?.menu_open {
            if dismissed == DISMISS_TRIES {
                return Err(Failure::Harness(
                    "Escape never closed the menu the scenario left open".to_owned(),
                ));
            }
            hands::key(self, "Escape")?;
            hands::frames(self, EDIT_SETTLE)?;
            dismissed += 1;
        }
        while self.snapshot()?.undoable > before.undoable {
            self.undo()?;
        }
        Ok(())
    }

    fn ask<Answer: DeserializeOwned>(
        &mut self,
        question: &EditorFrameQuestion,
    ) -> Result<Answer, Failure> {
        let answer = match self.probe.ask(question) {
            Ok(answer) => answer,
            Err(problem) => return Err(self.died_or(problem)),
        };
        serde_json::from_str(&answer).map_err(|error| {
            Failure::Harness(format!("an unreadable answer to {question:?}: {error}"))
        })
    }

    /// Reads the frames drawn since the last read into the measured window, when one is open and
    /// a read is due, or `now`.
    fn read_frames(&mut self, now: bool) -> Result<(), Failure> {
        let Some(measuring) = &self.measuring else {
            return Ok(());
        };
        if !now && measuring.last_read.elapsed() < READ_EVERY {
            return Ok(());
        }
        let since = measuring.since;
        let frames: ProbeFrames = self.ask(&EditorFrameQuestion::Frames { since })?;
        let measuring = self.measuring.as_mut().expect("measuring");
        measuring.last_read = Instant::now();
        if frames.dropped > 0 {
            return Err(Failure::Harness(format!(
                "the probe's ring of frames wrapped: {} frames dropped before they were read",
                frames.dropped
            )));
        }
        if let Some(last) = frames.samples.last() {
            measuring.since = last.frame;
            self.frame = self.frame.max(last.frame);
        }
        measuring.window.note_slow(&frames.slow);
        measuring.window.note_timed(frames.passes);
        measuring.window.extend(frames.samples);
        Ok(())
    }

    fn died_or(&mut self, problem: String) -> Failure {
        match self.exited() {
            Some(status) => Failure::Crashed(format!(
                "the editor exited with {status}; its log ends:\n{}",
                self.log_tail()
            )),
            None => Failure::Harness(format!("{problem}; its log ends:\n{}", self.log_tail())),
        }
    }

    fn log_tail(&self) -> String {
        let log = std::fs::read_to_string(self.folder.path().join(EDITOR_LOG)).unwrap_or_default();
        let lines: Vec<&str> = log.lines().collect();
        lines[lines.len().saturating_sub(LOG_TAIL)..].join("\n")
    }
}

impl Driven for EditorDriver {
    type Snapshot = EditorSnapshot;

    fn snapshot(&mut self) -> Result<EditorSnapshot, Failure> {
        EditorDriver::snapshot(self)
    }

    fn input(snapshot: &EditorSnapshot) -> &ProbeInput {
        &snapshot.input
    }

    fn frame_of(snapshot: &EditorSnapshot) -> u32 {
        snapshot.frame
    }

    fn seen(&self) -> ProbeInputSeen {
        self.seen
    }

    fn cursor(&self) -> Option<Pos<WindowPx>> {
        self.cursor
    }

    fn frame(&self) -> u32 {
        self.frame
    }

    fn display(&self) -> &str {
        self.desktop.display()
    }

    fn note(&mut self, step: &str) {
        let _ = writeln!(self.steps, "editor frame {}: {step}", self.frame);
    }
}

impl Drop for EditorDriver {
    fn drop(&mut self) {
        self.end();
    }
}

pub const CTRL: KeyModifiers = KeyModifiers {
    ctrl: true,
    shift: false,
    alt: false,
    super_key: false,
};
/// The measured window being filled: the frames read so far and where the next read starts.
struct Measuring {
    window: Window,
    since: u32,
    last_read: Instant,
}

impl Measuring {
    fn since(frame: u32) -> Measuring {
        Measuring {
            window: Window::default(),
            since: frame,
            last_read: Instant::now(),
        }
    }
}

const EDITOR_LOG: &str = "editor.log";
const FRAME_LOG: &str = "frames.jsonl";
const LOG_TAIL: usize = 40;
/// How long the editor has to come up, and to open the game and stand its area.
const LONG: Duration = Duration::from_secs(300);
/// How often the probe's ring of frames is read while measuring: well within what it holds at
/// any frame rate the editor reaches.
const READ_EVERY: Duration = Duration::from_secs(2);
/// Snapshots in a row the camera must hold still in to count as at rest.
const REST_SNAPSHOTS: u32 = 4;
/// Frames an input has to have its edit show in the history.
const EDIT_SETTLE: u32 = 6;
const DISMISS_TRIES: u32 = 4;
