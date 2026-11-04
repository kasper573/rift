//! Plays a scenario's script on a desktop client as a player would: every input goes in through
//! the X server and is confirmed through the probe's input counters before the next, targets in
//! the interface come from the probe's `ui` rects, targets in the world are projected through the
//! view the probe reports, and the camera is steered through the game's own bindings. The frames
//! come from the client's own clock, read from the probe's ring every few seconds, so the pace of
//! the input only paces the script.

use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use engine::core::content::Content;
use engine::core::math::{Pos, Rect, Size};
use engine::core::render::transition::ScreenTransitionPhase;
use engine::core::units::{Degrees, Meters, Radians, WindowPx};
use engine::systems::input::map::{InputAction, InputBinding, InputMap, KeyModifiers};
use engine::systems::probe::{
    ProbeFrameQuestion, ProbeFrames, ProbeInputSeen, ProbeSnapshot, ProbeTerrain, ProbeUiElement,
    ProbeView,
};
use engine::systems::staging::Staging;

use super::frames::{Video, Window};
use super::playtest::Playtest;
use super::scenario::{Button, CameraOp, Cond, Ground, Point, Scenario, Step};

/// A desktop client on a server, played through its window.
pub struct Driver {
    play: Playtest,
    inputs: InputMap,
    seen: ProbeInputSeen,
    cursor: Option<Pos<WindowPx>>,
    frame: u32,
    screen: Option<Size<WindowPx>>,
    terrain: Option<(String, ProbeTerrain)>,
    measuring: Option<Measuring>,
    steps: File,
}

/// Why a scenario could not be played through.
#[derive(Clone, Debug)]
pub enum Failure {
    /// An input never confirmed, a target never found, a wait run out, or the probe's ring of
    /// frames wrapped before it was read.
    Harness(String),
    /// The client or the server died.
    Crashed(String),
}

/// What a scenario measured: its frames, and how long the area took to stand after each move.
pub struct Measured {
    pub window: Window,
    pub stood_after_frames: Vec<u32>,
}

impl Driver {
    pub fn start(
        game: &Path,
        content: &Content,
        video: Video,
        folder: &Path,
    ) -> Result<Driver, Failure> {
        let play = Playtest::start(game, content, video.preferences()).map_err(Failure::Crashed)?;
        let steps = File::create(folder.join("steps.log"))
            .map_err(|error| Failure::Harness(format!("a steps log: {error}")))?;
        let mut driver = Driver {
            play,
            inputs: InputMap::new(content),
            seen: ProbeInputSeen::default(),
            cursor: None,
            frame: 0,
            screen: None,
            terrain: None,
            measuring: None,
            steps,
        };
        driver.wait_for("the player in the world", LONG, |snapshot| {
            (snapshot.me.is_some() && snapshot.transition == ScreenTransitionPhase::Idle)
                .then_some(())
        })?;
        Ok(driver)
    }

    pub fn adapter(&mut self) -> Result<Option<String>, Failure> {
        Ok(self.snapshot()?.adapter)
    }

    pub fn screen(&mut self) -> Result<Size<WindowPx>, Failure> {
        let snapshot = self.snapshot()?;
        snapshot
            .screen
            .ok_or_else(|| Failure::Harness("the client reports no screen".to_owned()))
    }

    /// Stages, sets up, and plays `scenario`, measuring its script.
    pub fn run(&mut self, scenario: &Scenario) -> Result<Measured, Failure> {
        self.note(&format!("=== {}", scenario.name));
        for staging in &scenario.stage {
            self.stage(staging)?;
        }
        for step in &scenario.setup {
            self.step(step)?;
        }
        let start = self.snapshot()?.frame;
        self.measuring = Some(Measuring {
            window: Window::default(),
            since: start,
            last_read: Instant::now(),
            stood_after_frames: Vec::new(),
            arrival_since: None,
        });
        let outcome = self.play_script(scenario);
        let measuring = self.measuring.take().expect("measuring");
        outcome?;
        Ok(Measured {
            window: measuring.window.after_warm_up(scenario.warm_up),
            stood_after_frames: measuring.stood_after_frames,
        })
    }

    /// Plays the script over and over until the scenario's duration has passed, then through to
    /// its end.
    fn play_script(&mut self, scenario: &Scenario) -> Result<(), Failure> {
        let began = Instant::now();
        loop {
            for step in &scenario.script {
                self.step(step)?;
            }
            self.read_frames(true)?;
            if began.elapsed().as_secs_f32() >= scenario.duration.0 {
                return Ok(());
            }
        }
    }

    fn stage(&mut self, staging: &Staging) -> Result<(), Failure> {
        self.note(&format!("stage {staging:?}"));
        self.play.stage(staging).map_err(|problem| {
            Failure::Harness(format!("the server didn't stage {staging:?}: {problem}"))
        })
    }

    fn step(&mut self, step: &Step) -> Result<(), Failure> {
        match step {
            Step::Move(point) => {
                let to = self.locate(point)?;
                self.move_pointer(to)?;
            }
            Step::Sweep(points) => {
                let mut through = Vec::with_capacity(points.len());
                for point in points {
                    through.push(self.locate(point)?);
                }
                self.sweep(&through)?;
            }
            Step::Click { button, times } => {
                self.click(*button, *times)?;
            }
            Step::Key(action) => {
                self.act(*action)?;
            }
            Step::Wait(cond) => self.wait(cond)?,
            Step::Repeat { times, steps } => {
                for _ in 0..*times {
                    for step in steps {
                        self.step(step)?;
                    }
                }
            }
            Step::SweepInterface => {
                let snapshot = self.snapshot()?;
                let through: Vec<Pos<WindowPx>> = snapshot
                    .ui
                    .iter()
                    .filter(|element| !element.in_world)
                    .map(|element| element.rect.center())
                    .collect();
                if !through.is_empty() {
                    self.sweep(&through)?;
                }
            }
            Step::Camera(op) => self.camera(*op)?,
            Step::Stage(staging) => {
                self.stage(staging)?;
                if let Staging::Place { .. } = staging
                    && let Some(measuring) = &mut self.measuring
                {
                    measuring.arrival_since = Some(self.frame);
                }
            }
        }
        Ok(())
    }

    fn wait(&mut self, cond: &Cond) -> Result<(), Failure> {
        match cond {
            Cond::Frames(frames) => {
                let until = self.frame + frames;
                self.idle_until(|driver| driver.frame >= until, LONG)
            }
            Cond::Seconds(seconds) => {
                let until = Instant::now() + Duration::from_secs_f32(seconds.0);
                self.idle_until(|_| Instant::now() >= until, LONG)
            }
            Cond::Arrived(place) => {
                let place = *place;
                self.wait_for("the arrival", LONG, move |snapshot| {
                    (snapshot.area == Some(place.area)
                        && snapshot.me.is_some()
                        && snapshot.transition == ScreenTransitionPhase::Idle)
                        .then_some(())
                })?;
                Ok(())
            }
            Cond::Stood => {
                let seen = self.wait_for("the scenery to stand", LONG, |snapshot| {
                    (!snapshot.scenery_standing).then_some(snapshot.frame)
                })?;
                if let Some(measuring) = &mut self.measuring
                    && let Some(since) = measuring.arrival_since.take()
                {
                    measuring
                        .stood_after_frames
                        .push(seen.saturating_sub(since));
                }
                Ok(())
            }
            Cond::AtRest => {
                self.wait_for("the player to come to rest", LONG, |snapshot| {
                    snapshot
                        .me
                        .as_ref()
                        .is_some_and(|me| me.resting)
                        .then_some(())
                })?;
                Ok(())
            }
            Cond::Settled => {
                let mut last: Vec<ProbeUiElement> = Vec::new();
                self.wait_for("the interface to settle", BUDGET, move |snapshot| {
                    let interface: Vec<ProbeUiElement> = snapshot
                        .ui
                        .iter()
                        .filter(|element| !element.in_world)
                        .cloned()
                        .collect();
                    let settled = interface == last;
                    last = interface;
                    settled.then_some(())
                })?;
                Ok(())
            }
        }
    }

    /// Lets the client play on, asking it only for its frames, until `done`.
    fn idle_until(
        &mut self,
        mut done: impl FnMut(&Driver) -> bool,
        budget: Duration,
    ) -> Result<(), Failure> {
        let since = Instant::now();
        loop {
            self.read_frames(false)?;
            self.check_alive()?;
            if done(self) {
                return Ok(());
            }
            if since.elapsed() > budget {
                return Err(Failure::Harness("a wait never ended".to_owned()));
            }
            std::thread::sleep(IDLE_SLICE);
        }
    }

    /// What the client shows in a frame after asking, with the frames read when they are due.
    pub fn snapshot(&mut self) -> Result<ProbeSnapshot, Failure> {
        self.read_frames(false)?;
        let answer = self.play.ask(&ProbeFrameQuestion::Snapshot);
        let mut snapshot: ProbeSnapshot = match answer {
            Ok(snapshot) => snapshot,
            Err(problem) => return Err(self.died_or(problem)),
        };
        match (&snapshot.terrain, &self.terrain) {
            (Some(terrain), _) => {
                self.terrain = snapshot
                    .area
                    .map(|area| (area.path(self.play.content()).to_owned(), terrain.clone()));
            }
            (None, Some((area, terrain)))
                if snapshot.area.map(|now| now.path(self.play.content())) == Some(area) =>
            {
                snapshot.terrain = Some(terrain.clone());
            }
            _ => {}
        }
        self.seen = snapshot.input.seen;
        self.cursor = snapshot.input.cursor;
        self.frame = snapshot.frame;
        self.screen = snapshot.screen;
        Ok(snapshot)
    }

    /// Reads the frames drawn since the last read into the measured window, when one is open and
    /// a read is due, or `now`.
    fn read_frames(&mut self, now: bool) -> Result<(), Failure> {
        let Some(measuring) = &mut self.measuring else {
            return Ok(());
        };
        if !now && measuring.last_read.elapsed() < READ_EVERY {
            return Ok(());
        }
        let since = measuring.since;
        let answer: Result<ProbeFrames, String> =
            self.play.ask(&ProbeFrameQuestion::Frames { since });
        let frames = match answer {
            Ok(frames) => frames,
            Err(problem) => return Err(self.died_or(problem)),
        };
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
        measuring.window.note_worst(&frames.worst);
        measuring.window.extend(frames.samples);
        Ok(())
    }

    fn died_or(&mut self, problem: String) -> Failure {
        match self.play.exited() {
            Some(status) => Failure::Crashed(format!(
                "the client exited with {status}; its log ends:\n{}",
                self.play.log_tail()
            )),
            None => Failure::Harness(format!(
                "{problem}; its log ends:\n{}",
                self.play.log_tail()
            )),
        }
    }

    fn check_alive(&mut self) -> Result<(), Failure> {
        match self.play.exited() {
            Some(status) => Err(Failure::Crashed(format!(
                "the client exited with {status}; its log ends:\n{}",
                self.play.log_tail()
            ))),
            None => Ok(()),
        }
    }

    pub fn wait_for<T>(
        &mut self,
        what: &str,
        budget: Duration,
        mut found: impl FnMut(&ProbeSnapshot) -> Option<T>,
    ) -> Result<T, Failure> {
        let since = Instant::now();
        let first = self.frame;
        loop {
            let snapshot = self.snapshot()?;
            if let Some(done) = found(&snapshot) {
                return Ok(done);
            }
            if since.elapsed() >= budget && self.frame.saturating_sub(first) >= FAST_FRAMES {
                return Err(Failure::Harness(format!("gave up waiting for {what}")));
            }
        }
    }

    fn locate(&mut self, point: &Point) -> Result<Pos<WindowPx>, Failure> {
        match point {
            Point::Screen { across, down } => {
                let screen = self.screen()?;
                Ok(Pos::new(screen.width * across, screen.height * down))
            }
            Point::Ground(ground) => {
                let snapshot = self.snapshot()?;
                let at = match *ground {
                    Ground::NearMe { dx, dy } => {
                        let me = snapshot
                            .me
                            .as_ref()
                            .ok_or_else(|| {
                                Failure::Harness("the player is not in the world".to_owned())
                            })?
                            .at;
                        Pos::new(me.x + dx.0, me.y + dy.0)
                    }
                };
                clickable_point(&snapshot, at).ok_or_else(|| {
                    Failure::Harness(format!("the ground at {:?} is out of view", (at.x, at.y)))
                })
            }
        }
    }

    fn camera(&mut self, op: CameraOp) -> Result<(), Failure> {
        match op {
            CameraOp::Turn(by) => {
                let (button, held) = self.drag_binding(InputAction::TurnCamera)?;
                self.steer(
                    button,
                    &held,
                    by,
                    |view| view.yaw,
                    Nudge::across(STEER_STEP),
                )
            }
            CameraOp::Tilt(by) => {
                let (button, held) = self.drag_binding(InputAction::TiltCamera)?;
                self.steer(
                    button,
                    &held,
                    by,
                    |view| view.pitch,
                    Nudge::down(STEER_STEP),
                )
            }
            CameraOp::Zoom(steps) => {
                let here = self.locate(&Point::Ground(Ground::NearMe {
                    dx: Meters(0.0),
                    dy: Meters(0.0),
                }))?;
                self.move_pointer(here)?;
                let mut zoom = self.snapshot()?.view.map(|view| view.zoom);
                let mut moved = 0;
                while moved < steps.unsigned_abs() {
                    self.wheel(-steps.signum())?;
                    let now = self.snapshot()?.view.map(|view| view.zoom);
                    match (zoom, now) {
                        (Some(was), Some(is)) if was != is => moved += u32::from(was.abs_diff(is)),
                        _ => break,
                    }
                    zoom = now;
                }
                if moved == 0 {
                    return Err(Failure::Harness(
                        "a click of the wheel didn't zoom the camera".to_owned(),
                    ));
                }
                Ok(())
            }
            CameraOp::Home => {
                self.act(InputAction::CameraHome)?;
                self.wait_for("the camera home", BUDGET, |snapshot| {
                    snapshot
                        .view
                        .as_ref()
                        .is_some_and(|view| view.home)
                        .then_some(())
                })?;
                Ok(())
            }
        }
    }

    /// Holds `button` with `held` over the ground the player stands on and moves the pointer a
    /// nudge at a time until the angle `read` of the view has come round by `by`.
    fn steer(
        &mut self,
        button: Button,
        held: &KeyModifiers,
        by: Degrees,
        read: impl Fn(&ProbeView) -> Radians,
        nudge: Nudge,
    ) -> Result<(), Failure> {
        let here = self.locate(&Point::Ground(Ground::NearMe {
            dx: Meters(0.0),
            dy: Meters(0.0),
        }))?;
        let seen = self.move_pointer(here)?;
        let mut last = seen
            .view
            .as_ref()
            .map(&read)
            .ok_or_else(|| Failure::Harness("no view to steer".to_owned()))?;
        let wanted = by.radians().0.abs();
        let mut turned = 0.0;
        let modifiers = modifier_keys(held);
        for key in &modifiers {
            self.key_down(key)?;
        }
        self.button_down(button)?;
        let mut at = here;
        let steps = (wanted / Degrees(1.0).radians().0 * 4.0).ceil() as u32 + 8;
        let mut outcome = Ok(());
        let mut stalled = 0;
        for _ in 0..steps.max(STEER_LEAST_STEPS) {
            at = nudged(at, if by.0 >= 0.0 { nudge } else { nudge.back() });
            let Some(screen) = self.screen else { break };
            if at.x < 1.0 || at.y < 1.0 || at.x >= screen.width - 1.0 || at.y >= screen.height - 1.0
            {
                at = here;
                self.button_up(button)?;
                self.move_pointer(at)?;
                self.button_down(button)?;
            }
            let seen = match self.move_pointer(at) {
                Ok(seen) => seen,
                Err(failure) => {
                    outcome = Err(failure);
                    break;
                }
            };
            if let Some(view) = &seen.view {
                let now = read(view);
                let moved = wrapped(now.0 - last.0).abs();
                turned += moved;
                last = now;
                stalled = if moved > 0.0 { 0 } else { stalled + 1 };
            }
            if turned >= wanted || (turned > 0.0 && stalled >= STEER_STALLED) {
                break;
            }
        }
        self.button_up(button)?;
        for key in modifiers.iter().rev() {
            self.key_up(key)?;
        }
        outcome?;
        if turned == 0.0 {
            return Err(Failure::Harness(format!(
                "the camera never moved under a drag of {:.0}°",
                by.0.abs()
            )));
        }
        Ok(())
    }

    fn drag_binding(&self, action: InputAction) -> Result<(Button, KeyModifiers), Failure> {
        self.inputs
            .bindings(action)
            .iter()
            .find_map(|binding| match binding {
                InputBinding::Drag(drag) => Some((button_of(drag.button), drag.modifiers)),
                InputBinding::Mouse { button, modifiers } => Some((button_of(*button), *modifiers)),
                _ => None,
            })
            .ok_or_else(|| Failure::Harness(format!("no drag is bound to {action:?}")))
    }

    /// Presses the key the action is bound to, with its modifiers held.
    fn act(&mut self, action: InputAction) -> Result<ProbeSnapshot, Failure> {
        let gesture = self
            .inputs
            .bindings(action)
            .iter()
            .find_map(|binding| match binding {
                InputBinding::Key(gesture) => Some(*gesture),
                _ => None,
            })
            .ok_or_else(|| Failure::Harness(format!("no key is bound to {action:?}")))?;
        let modifiers = modifier_keys(&gesture.modifiers);
        for key in &modifiers {
            self.key_down(key)?;
        }
        let landed = self.key(&keysym(&format!("{:?}", gesture.key)));
        for key in modifiers.iter().rev() {
            self.key_up(key)?;
        }
        landed
    }

    pub fn move_pointer(&mut self, to: Pos<WindowPx>) -> Result<ProbeSnapshot, Failure> {
        let to = Pos::new(to.x.round(), to.y.round());
        let moves = self.mouse_moves(&[to], None);
        self.xdotool(&moves)?;
        let seen = self.wait_for("the pointer to arrive", BUDGET, move |snapshot| {
            let cursor = snapshot.input.cursor?;
            (cursor.distance_to(to) <= 1.0).then(|| snapshot.clone())
        })?;
        self.note(&format!("pointer at {:?}", (to.x, to.y)));
        Ok(seen)
    }

    fn sweep(&mut self, through: &[Pos<WindowPx>]) -> Result<ProbeSnapshot, Failure> {
        let Some(last) = through.last() else {
            return self.snapshot();
        };
        let to = Pos::new(last.x.round(), last.y.round());
        let through: Vec<Pos<WindowPx>> = through
            .iter()
            .map(|at| Pos::new(at.x.round(), at.y.round()))
            .collect();
        let moves = self.mouse_moves(&through, Some(SWEEP_STEP));
        self.xdotool(&moves)?;
        let seen = self.wait_for("the pointer to come to rest", BUDGET, move |snapshot| {
            let cursor = snapshot.input.cursor?;
            (cursor.distance_to(to) <= 1.0).then(|| snapshot.clone())
        })?;
        self.note(&format!("pointer swept to {:?}", (to.x, to.y)));
        Ok(seen)
    }

    fn click(&mut self, button: Button, times: u8) -> Result<ProbeSnapshot, Failure> {
        let before = self.seen;
        let number = button.x_number().to_string();
        let times_arg = times.to_string();
        self.xdotool(&[
            "click".to_owned(),
            "--repeat".to_owned(),
            times_arg,
            "--delay".to_owned(),
            CLICK_APART_MS.to_owned(),
            number,
        ])?;
        let landed = self.wait_for("the click to land", BUDGET, move |snapshot| {
            let seen = &snapshot.input.seen;
            (seen.mouse_presses >= before.mouse_presses + u32::from(times)
                && seen.mouse_releases >= before.mouse_releases + u32::from(times))
            .then(|| snapshot.clone())
        })?;
        self.note(&format!("click {times}× button {}", button.x_number()));
        Ok(landed)
    }

    fn button_down(&mut self, button: Button) -> Result<ProbeSnapshot, Failure> {
        let before = self.seen;
        self.xdotool(&["mousedown".to_owned(), button.x_number().to_string()])?;
        self.wait_for("the button to go down", BUDGET, move |snapshot| {
            (snapshot.input.seen.mouse_presses > before.mouse_presses).then(|| snapshot.clone())
        })
    }

    fn button_up(&mut self, button: Button) -> Result<ProbeSnapshot, Failure> {
        let before = self.seen;
        self.xdotool(&["mouseup".to_owned(), button.x_number().to_string()])?;
        self.wait_for("the button to come up", BUDGET, move |snapshot| {
            (snapshot.input.seen.mouse_releases > before.mouse_releases).then(|| snapshot.clone())
        })
    }

    /// X turns the wheel by clicking buttons 4 (up) and 5 (down), a line per click.
    fn wheel(&mut self, lines: i32) -> Result<ProbeSnapshot, Failure> {
        let before = self.seen;
        let button = if lines < 0 { "4" } else { "5" };
        let turns = lines.unsigned_abs();
        self.xdotool(&[
            "click".to_owned(),
            "--repeat".to_owned(),
            turns.to_string(),
            "--delay".to_owned(),
            WHEEL_APART_MS.to_owned(),
            button.to_owned(),
        ])?;
        self.wait_for("the wheel to turn", BUDGET, move |snapshot| {
            (snapshot.input.seen.wheel_turns >= before.wheel_turns + turns).then_some(())
        })?;
        // winit reports a synthetic wheel button's release as a line as it does its press, so a
        // click scrolls twice, the second time a frame or two after the first is seen.
        self.wait(&Cond::Frames(2))?;
        self.note(&format!("wheel {lines} lines"));
        self.snapshot()
    }

    fn key(&mut self, keysym: &str) -> Result<ProbeSnapshot, Failure> {
        let before = self.seen;
        self.xdotool(&["key".to_owned(), keysym.to_owned()])?;
        let landed = self.wait_for("the key press to land", BUDGET, move |snapshot| {
            (snapshot.input.seen.key_presses > before.key_presses).then(|| snapshot.clone())
        })?;
        self.note(&format!("key {keysym}"));
        Ok(landed)
    }

    fn key_down(&mut self, keysym: &str) -> Result<ProbeSnapshot, Failure> {
        let before = self.seen;
        self.xdotool(&["keydown".to_owned(), keysym.to_owned()])?;
        self.wait_for("the key to go down", BUDGET, move |snapshot| {
            (snapshot.input.seen.key_presses > before.key_presses).then(|| snapshot.clone())
        })
    }

    /// Releases count nowhere, but X delivers them before anything sent after.
    fn key_up(&mut self, keysym: &str) -> Result<(), Failure> {
        self.xdotool(&["keyup".to_owned(), keysym.to_owned()])
    }

    /// The xdotool commands that move the pointer through each of `points` as a mouse does, with
    /// a `pause` after each when given. X reports the mouse moving only for a move by an offset,
    /// and lands that move where its pointer acceleration takes it, so each goes by its offset and
    /// is then set on its point.
    fn mouse_moves(&self, points: &[Pos<WindowPx>], pause: Option<&str>) -> Vec<String> {
        let mut last = self.cursor;
        let mut moves = Vec::new();
        for at in points {
            if let Some(from) = last {
                let (dx, dy) = (at.x - from.x, at.y - from.y);
                moves.extend(["mousemove_relative".to_owned(), "--".to_owned()]);
                moves.extend([dx.round().to_string(), dy.round().to_string()]);
            }
            moves.extend(["mousemove".to_owned(), at.x.to_string(), at.y.to_string()]);
            if let Some(pause) = pause {
                moves.extend(["sleep".to_owned(), pause.to_owned()]);
            }
            last = Some(*at);
        }
        moves
    }

    fn xdotool(&mut self, args: &[String]) -> Result<(), Failure> {
        let run = Command::new("xdotool")
            .env("DISPLAY", self.play.display())
            .args(args)
            .output()
            .map_err(|error| Failure::Harness(format!("xdotool doesn't run: {error}")))?;
        if run.status.success() {
            Ok(())
        } else {
            Err(Failure::Harness(format!(
                "xdotool {args:?}: {}",
                String::from_utf8_lossy(&run.stderr)
            )))
        }
    }

    fn note(&mut self, step: &str) {
        let _ = writeln!(self.steps, "frame {}: {step}", self.frame);
    }
}

/// The measured window being filled: the frames read so far, where the next read starts, and
/// the arrival being timed.
struct Measuring {
    window: Window,
    since: u32,
    last_read: Instant,
    stood_after_frames: Vec<u32>,
    arrival_since: Option<u32>,
}

/// How long a wait lasts, in time; a fast machine must also have drawn the frames it would in
/// that time, so a slow machine gets as many frames as a fast one.
const BUDGET: Duration = Duration::from_secs(20);
const LONG: Duration = Duration::from_secs(180);
const FAST_FRAMES: u32 = 20 * 60;
/// How often the probe's ring of frames is read while measuring: well within what it holds at
/// any frame rate.
const READ_EVERY: Duration = Duration::from_secs(2);
const IDLE_SLICE: Duration = Duration::from_millis(50);
/// Window pixels the pointer moves per step of a steer.
const STEER_STEP: f32 = 24.0;
const STEER_LEAST_STEPS: u32 = 16;
/// Nudges the camera may stand still under before the steer counts as having reached a limit.
const STEER_STALLED: u32 = 6;
const CLICK_APART_MS: &str = "40";
const WHEEL_APART_MS: &str = "10";
/// Seconds between the moves of a sweep: a mouse reports a few hundred times a second.
const SWEEP_STEP: &str = "0.004";

/// How far the pointer moves across and down the window.
#[derive(Clone, Copy, Debug)]
struct Nudge {
    dx: WindowPx,
    dy: WindowPx,
}

impl Nudge {
    fn across(dx: f32) -> Nudge {
        Nudge {
            dx: WindowPx(dx),
            dy: WindowPx(0.0),
        }
    }

    fn down(dy: f32) -> Nudge {
        Nudge {
            dx: WindowPx(0.0),
            dy: WindowPx(dy),
        }
    }

    fn back(self) -> Nudge {
        Nudge {
            dx: WindowPx(-self.dx.0),
            dy: WindowPx(-self.dy.0),
        }
    }
}

fn nudged(at: Pos<WindowPx>, by: Nudge) -> Pos<WindowPx> {
    Pos::new(at.x + by.dx.0, at.y + by.dy.0)
}

/// An angle's difference wrapped into a half turn either way.
fn wrapped(radians: f32) -> f32 {
    use std::f32::consts::PI;
    (radians + PI).rem_euclid(2.0 * PI) - PI
}

/// Where the ground at `at` shows in the window, when it does and nothing of the interface
/// covers it.
pub fn clickable_point(snapshot: &ProbeSnapshot, at: Pos<Meters>) -> Option<Pos<WindowPx>> {
    let view: &ProbeView = snapshot.view.as_ref()?;
    let height = snapshot
        .terrain
        .as_ref()
        .map_or(Meters(0.0), |terrain: &ProbeTerrain| terrain.height_at(at));
    let point = view.window_point(at, height).round();
    let window = Rect::from_size(snapshot.screen?);
    let covered = snapshot.covered.iter().any(|cover| cover.contains(point));
    (window.contains(point) && !covered).then_some(point)
}

fn button_of(button: bevy::input::mouse::MouseButton) -> Button {
    use bevy::input::mouse::MouseButton;
    match button {
        MouseButton::Left => Button::Left,
        MouseButton::Middle => Button::Middle,
        MouseButton::Right => Button::Right,
        MouseButton::Back => Button::Back,
        MouseButton::Forward => Button::Forward,
        MouseButton::Other(number) => Button::Other(number),
    }
}

fn modifier_keys(held: &KeyModifiers) -> Vec<&'static str> {
    [
        (held.ctrl, "Control_L"),
        (held.shift, "Shift_L"),
        (held.alt, "Alt_L"),
        (held.super_key, "Super_L"),
    ]
    .into_iter()
    .filter_map(|(down, key)| down.then_some(key))
    .collect()
}

/// xdotool names keys by their X keysyms; bevy by its `KeyCode` names.
fn keysym(key: &str) -> String {
    match key {
        "Space" => "space".to_owned(),
        "Enter" => "Return".to_owned(),
        "Backspace" => "BackSpace".to_owned(),
        "PageUp" => "Prior".to_owned(),
        "PageDown" => "Next".to_owned(),
        _ if key.starts_with("Key") => key["Key".len()..].to_lowercase(),
        _ if key.starts_with("Digit") => key["Digit".len()..].to_owned(),
        _ if key.starts_with("Arrow") => key["Arrow".len()..].to_owned(),
        _ => key.to_owned(),
    }
}
