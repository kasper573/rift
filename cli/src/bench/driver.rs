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

use engine::core::content::{Content, ContentId};
use engine::core::math::{Pos, Rect, Size};
use engine::core::render::transition::ScreenTransitionPhase;
use engine::core::units::{Degrees, MeterPos, Meters, Radians, WindowPx};
use engine::systems::input::camera::DRAG_TURN;
use engine::systems::input::map::{InputAction, InputBinding, InputMap, KeyModifiers};
use engine::systems::player::snapshot::CharacterSnapshot;
use engine::systems::probe::{
    ProbeFrameQuestion, ProbeFrames, ProbeInputSeen, ProbeSnapshot, ProbeTerrain, ProbeUiElement,
    ProbeView,
};
use engine::systems::staging::Staging;

use super::frames::Window;
use super::playtest::Playtest;
use super::scenario::{Button, CameraOp, Cond, Ground, Point, Scenario, Step, UiTarget};
use crate::render::VideoPreset;

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
    /// A client on a server of its own, every step it is put through noted in the log at
    /// `steps`, which a run's drivers share.
    pub fn start(
        game: &Path,
        content: &Content,
        character: Option<ContentId<CharacterSnapshot>>,
        video: VideoPreset,
        steps: &Path,
    ) -> Result<Driver, Failure> {
        let play = Playtest::start(game, content, character, video.preferences())
            .map_err(Failure::Crashed)?;
        let steps = File::options()
            .append(true)
            .create(true)
            .open(steps)
            .map_err(|error| Failure::Harness(format!("{}: {error}", steps.display())))?;
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

    /// Where the server the client plays on takes requests.
    pub fn server_url(&self) -> String {
        self.play.server_url()
    }

    /// Stages, sets up, and plays `scenario`, measuring its script, with nothing left open over
    /// the world by the scenario before, and the scene staged standing before the setup plays.
    pub fn run(&mut self, scenario: &Scenario) -> Result<Measured, Failure> {
        self.note(&format!("=== {}", scenario.name));
        self.close_windows()?;
        for staging in &scenario.stage {
            self.stage(staging)?;
            if let Staging::Place { place } = staging {
                self.wait(&Cond::Arrived(*place))?;
                self.wait(&Cond::Stood)?;
            }
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

    /// Stages a walk to `to` and waits for the player to set off, asking again when they don't:
    /// the server drops a walk it finds no route for, as over ground it hasn't laid yet.
    fn walk(&mut self, to: Pos<Meters>) -> Result<(), Failure> {
        let since = Instant::now();
        loop {
            self.stage(&Staging::Walk { to })?;
            let from = self.me_at()?;
            if from.distance(to) <= STILL_WITHIN {
                return Ok(());
            }
            let asked = Instant::now();
            while asked.elapsed() < SET_OFF_WITHIN {
                if self.me_at()?.distance(from) > STILL_WITHIN {
                    return Ok(());
                }
                std::thread::sleep(IDLE_SLICE);
            }
            if since.elapsed() > BUDGET {
                let me = self.snapshot()?.me;
                return Err(Failure::Harness(format!(
                    "the player never set off for {:?}, standing at {:?} doing {:?}{}",
                    (to.x, to.y),
                    me.as_ref().map(|me| (me.at.x, me.at.y)),
                    me.as_ref().map(|me| me.action),
                    me.as_ref()
                        .and_then(|me| me.engaged)
                        .map_or(String::new(), |foe| format!(", engaged with {foe:?}"))
                )));
            }
            self.note("the player didn't set off: asking again");
        }
    }

    fn me_at(&mut self) -> Result<Pos<Meters>, Failure> {
        self.snapshot()?
            .me
            .map(|me| me.at)
            .ok_or_else(|| Failure::Harness("the player is not in the world".to_owned()))
    }

    /// Asks the server to stage `staging`, asking again for a while when it can't yet: what a
    /// scene needs may still be waking up around a player who just arrived.
    fn stage(&mut self, staging: &Staging) -> Result<(), Failure> {
        self.note(&format!("stage {staging:?}"));
        let since = Instant::now();
        loop {
            match self.play.stage(staging) {
                Ok(()) => return Ok(()),
                Err(problem) if since.elapsed() < BUDGET => {
                    self.check_alive()?;
                    std::thread::sleep(STAGE_RETRY);
                    self.note(&format!("the server didn't stage it yet: {problem}"));
                }
                Err(problem) => {
                    return Err(Failure::Harness(format!(
                        "the server didn't stage {staging:?}: {problem}"
                    )));
                }
            }
        }
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
            Step::Down(button) => {
                self.button_down(*button)?;
            }
            Step::Up(button) => {
                self.button_up(*button)?;
            }
            Step::Wheel(lines) => {
                self.wheel(*lines)?;
            }
            Step::Key(action) => {
                self.act(*action)?;
            }
            Step::Type(text) => self.type_text(text)?,
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
            Step::ClickUi(target) => {
                let element = self.find_ui(target)?;
                self.move_pointer(element.rect.center())?;
                self.click(Button::Left, 1)?;
            }
            Step::HoverUi(target) => {
                let element = self.find_ui(target)?;
                self.move_pointer(element.rect.center())?;
            }
            Step::DragUi { from, to } => {
                let from = self.find_ui(from)?.rect.center();
                let to = self.find_ui(to)?.rect.center();
                let (button, held) = self.mouse_binding(InputAction::CarryItem)?;
                self.drag(button, &held, from, to)?;
            }
            Step::DragWindows => self.drag_windows()?,
            Step::DragItems => {
                let slots = self.slots()?;
                if let [first, second, ..] = slots[..] {
                    let (button, held) = self.mouse_binding(InputAction::CarryItem)?;
                    self.drag(button, &held, first, second)?;
                } else {
                    self.note("no two item slots to drag between");
                }
            }
            Step::SlotOptions => {
                let slots = self.slots()?;
                if let Some(&slot) = slots.first() {
                    let (button, held) = self.mouse_binding(InputAction::SlotOptions)?;
                    self.move_pointer(slot)?;
                    self.hold(&held, |driver| driver.click(button, 1).map(drop))?;
                    self.wait(&Cond::Settled)?;
                } else {
                    self.note("no item slot to open the options of");
                }
            }
            Step::CloseWindows => self.close_windows()?,
            Step::Resize { width, height } => self.resize(*width, *height)?,
            Step::Camera(op) => self.camera(*op)?,
            Step::Stage(staging) => match staging {
                Staging::Walk { to } => self.walk(*to)?,
                Staging::Place { .. } => {
                    self.stage(staging)?;
                    if let Some(measuring) = &mut self.measuring {
                        measuring.arrival_since = Some(self.frame);
                    }
                }
                _ => self.stage(staging)?,
            },
            Step::CrossWarp => {
                let snapshot = self.snapshot()?;
                let me = snapshot
                    .me
                    .as_ref()
                    .ok_or_else(|| Failure::Harness("the player is not in the world".to_owned()))?
                    .at;
                let warp = snapshot
                    .warps
                    .iter()
                    .min_by(|a, b| a.at.distance(me).0.total_cmp(&b.at.distance(me).0))
                    .cloned()
                    .ok_or_else(|| Failure::Harness("the client shows no warp".to_owned()))?;
                self.stage(&Staging::Walk { to: warp.at })?;
                if let Some(measuring) = &mut self.measuring {
                    measuring.arrival_since = Some(self.frame);
                }
                self.wait_for("the warp to lead on", LONG, move |snapshot| {
                    (snapshot.area == Some(warp.to)
                        && snapshot.me.is_some()
                        && snapshot.transition == ScreenTransitionPhase::Idle)
                        .then_some(())
                })?;
            }
        }
        Ok(())
    }

    /// Dismisses whatever is open over the world, topmost first, until a dismissal closes
    /// nothing more: what the interface always shows stays.
    fn close_windows(&mut self) -> Result<(), Failure> {
        let mut covered = self.snapshot()?.covered.len();
        for _ in 0..CLOSE_TRIES {
            self.act(InputAction::Dismiss)?;
            self.wait(&Cond::Settled)?;
            let now = self.snapshot()?.covered.len();
            if now >= covered {
                return Ok(());
            }
            covered = now;
        }
        Err(Failure::Harness(format!(
            "the interface keeps changing under {CLOSE_TRIES} dismissals"
        )))
    }

    /// The element the target names, shown and in reach of the pointer: where no window shows
    /// it, each is scrolled down a little at a time until one does.
    fn find_ui(&mut self, target: &UiTarget) -> Result<ProbeUiElement, Failure> {
        self.wait(&Cond::Settled)?;
        for _ in 0..SCROLL_TRIES {
            let snapshot = self.snapshot()?;
            if let Some(element) = shown_element(&snapshot, target) {
                return Ok(element);
            }
            for cover in snapshot.covered {
                self.move_pointer(cover.center())?;
                self.wheel(SCROLL_LINES)?;
            }
            self.wait(&Cond::Settled)?;
        }
        Err(Failure::Harness(format!(
            "the interface never showed {target:?}"
        )))
    }

    /// Every slot of the interface showing an item, as the slot's picture.
    fn slots(&mut self) -> Result<Vec<Pos<WindowPx>>, Failure> {
        let snapshot = self.snapshot()?;
        Ok(snapshot
            .ui
            .iter()
            .filter(|element| {
                element.image.is_some() && !element.in_world && !element.click_through
            })
            .map(|element| element.rect.center())
            .collect())
    }

    /// Drags every window of the interface by its title a little way across the screen and back.
    fn drag_windows(&mut self) -> Result<(), Failure> {
        let snapshot = self.snapshot()?;
        let screen = snapshot
            .screen
            .ok_or_else(|| Failure::Harness("the client reports no screen".to_owned()))?;
        let mut titles = Vec::new();
        for cover in &snapshot.covered {
            let title = snapshot
                .ui
                .iter()
                .filter(|element| {
                    !element.in_world
                        && !element.editable
                        && !element.button
                        && element.text.is_some()
                        && cover.contains(element.rect.center())
                })
                .min_by(|a, b| a.rect.min().y.total_cmp(&b.rect.min().y));
            if let Some(title) = title {
                titles.push(title.rect.center());
            }
        }
        for from in titles {
            let to = Pos::new(
                (from.x + WINDOW_DRAG).min(screen.width - 2.0),
                (from.y + WINDOW_DRAG).min(screen.height - 2.0),
            );
            self.drag(Button::Left, &KeyModifiers::NONE, from, to)?;
            self.drag(Button::Left, &KeyModifiers::NONE, to, from)?;
        }
        Ok(())
    }

    /// Holds `button` with `held` from `from` and moves the pointer to `to` a step at a time.
    fn drag(
        &mut self,
        button: Button,
        held: &KeyModifiers,
        from: Pos<WindowPx>,
        to: Pos<WindowPx>,
    ) -> Result<(), Failure> {
        self.move_pointer(from)?;
        self.hold(held, |driver| {
            driver.button_down(button)?;
            for step in 1..=DRAG_STEPS {
                driver.move_pointer(from.lerp(to, step as f32 / DRAG_STEPS as f32))?;
            }
            driver.button_up(button).map(drop)
        })
    }

    /// Does `with` while the modifier keys of `held` are down.
    fn hold(
        &mut self,
        held: &KeyModifiers,
        with: impl FnOnce(&mut Driver) -> Result<(), Failure>,
    ) -> Result<(), Failure> {
        let modifiers = modifier_keys(held);
        for key in &modifiers {
            self.key_down(key)?;
        }
        let outcome = with(self);
        for key in modifiers.iter().rev() {
            self.key_up(key)?;
        }
        outcome
    }

    fn type_text(&mut self, text: &str) -> Result<(), Failure> {
        let before = self.seen;
        let typed = text.chars().count() as u32;
        self.xdotool(&[
            "type".to_owned(),
            "--delay".to_owned(),
            TYPE_APART_MS.to_owned(),
            "--".to_owned(),
            text.to_owned(),
        ])?;
        self.wait_for("the typing to land", BUDGET, move |snapshot| {
            (snapshot.input.seen.key_presses >= before.key_presses + typed).then_some(())
        })?;
        self.note(&format!("typed {text:?}"));
        Ok(())
    }

    /// Resizes the client's window as a window manager does, and waits for the client to draw at
    /// the new size.
    fn resize(&mut self, width: u32, height: u32) -> Result<(), Failure> {
        let pid = self.play.client_pid().to_string();
        self.xdotool(&[
            "search".to_owned(),
            "--sync".to_owned(),
            "--onlyvisible".to_owned(),
            "--pid".to_owned(),
            pid,
            "--name".to_owned(),
            ".".to_owned(),
            "windowsize".to_owned(),
            "--sync".to_owned(),
            width.to_string(),
            height.to_string(),
        ])?;
        self.wait_for("the window to take its size", BUDGET, move |snapshot| {
            let screen = snapshot.screen?;
            (screen.width as u32 == width && screen.height as u32 == height).then_some(())
        })?;
        self.note(&format!("resized to {width}×{height}"));
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
                let mut last: Option<(Pos<Meters>, Instant)> = None;
                let mut rested_at: Option<u32> = None;
                self.wait_for("the player to come to rest", LONG, move |snapshot| {
                    let me = snapshot.me.as_ref()?;
                    if me.resting {
                        let since = *rested_at.get_or_insert(snapshot.frame);
                        return (snapshot.frame >= since + REST_FRAMES).then_some(());
                    }
                    rested_at = None;
                    match last {
                        Some((at, since)) if at.distance(me.at) < STILL_WITHIN => {
                            (since.elapsed() > BUDGET).then_some(())
                        }
                        _ => {
                            last = Some((me.at, Instant::now()));
                            None
                        }
                    }
                })?;
                let me = self.snapshot()?.me;
                if me.is_some_and(|me| !me.resting) {
                    return Err(Failure::Harness(
                        "the player stopped moving without coming to rest".to_owned(),
                    ));
                }
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
                    Failure::Harness(format!(
                        "the ground at {:?} is {}",
                        (at.x, at.y),
                        hidden_by(&snapshot, at)
                    ))
                })
            }
        }
    }

    fn camera(&mut self, op: CameraOp) -> Result<(), Failure> {
        match op {
            CameraOp::Turn(by) => {
                let (button, held) = self.mouse_binding(InputAction::TurnCamera)?;
                self.steer(
                    button,
                    &held,
                    by,
                    |view| view.yaw,
                    Nudge::across(STEER_STEP),
                )
            }
            CameraOp::Tilt(by) => {
                let (button, held) = self.mouse_binding(InputAction::TiltCamera)?;
                self.steer(
                    button,
                    &held,
                    by,
                    |view| view.pitch,
                    Nudge::down(STEER_STEP),
                )
            }
            CameraOp::Zoom(steps) => {
                let here = self.open_ground()?;
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
            CameraOp::Face(to) => {
                let mut sign = 1.0;
                let mut last_off: Option<f32> = None;
                for _ in 0..FACE_TRIES {
                    self.camera_settled()?;
                    let off = self.ahead_of(to)?;
                    self.note(&format!(
                        "{:?} lies {off:.1}° off straight ahead",
                        (to.x, to.y)
                    ));
                    if off.abs() <= FACE_WITHIN {
                        return Ok(());
                    }
                    if let Some(last) = last_off
                        && off.abs() >= last.abs()
                    {
                        sign = -sign;
                    }
                    last_off = Some(off);
                    self.camera(CameraOp::Turn(Degrees(off * sign)))?;
                }
                Err(Failure::Harness(format!(
                    "the camera never came to face {:?}",
                    (to.x, to.y)
                )))
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

    /// Holds `button` with `held` over open ground near the player and moves the pointer as far
    /// as turning the camera `by` takes, a nudge at a time, then waits for the camera to settle;
    /// the camera stops short at its limits, and only one that never moved fails.
    fn steer(
        &mut self,
        button: Button,
        held: &KeyModifiers,
        by: Degrees,
        read: impl Fn(&ProbeView) -> Radians,
        nudge: Nudge,
    ) -> Result<(), Failure> {
        let here = self.open_ground()?;
        let seen = self.move_pointer(here)?;
        let before = seen
            .view
            .as_ref()
            .map(&read)
            .ok_or_else(|| Failure::Harness("no view to steer".to_owned()))?;
        let pixels = by.0.abs() / DRAG_TURN.0;
        let nudge = if by.0 >= 0.0 { nudge } else { nudge.back() };
        let pressed = self.drag_by(button, held, here, pixels, nudge)?;
        let after = self.camera_settled()?;
        if read(&after) != before {
            return Ok(());
        }
        let under = under_point(&pressed, here);
        self.move_pointer(here)?;
        self.drag_by(button, held, here, pixels, nudge)?;
        let again = self.camera_settled()?;
        let second = if read(&again) == before {
            "nor did a second"
        } else {
            "a second did"
        };
        Err(Failure::Harness(format!(
            "the camera never moved under a drag of {:.0}° pressed at {:?} over {under}; {second}",
            by.0.abs(),
            (here.x, here.y)
        )))
    }

    /// Holds `button` with `held` on the pointer, which stands at `here`, and moves it `pixels`
    /// along `nudge`, starting over from `here` where the window's edge stops it, then lets go:
    /// what the client showed as the button went down.
    fn drag_by(
        &mut self,
        button: Button,
        held: &KeyModifiers,
        here: Pos<WindowPx>,
        pixels: f32,
        nudge: Nudge,
    ) -> Result<ProbeSnapshot, Failure> {
        let modifiers = modifier_keys(held);
        for key in &modifiers {
            self.key_down(key)?;
        }
        let pressed = self.button_down(button)?;
        let mut at = here;
        let mut moved = 0.0;
        let mut outcome = Ok(());
        while moved < pixels {
            let step = (pixels - moved).min(STEER_STEP);
            at = nudged(at, nudge.scaled(step / STEER_STEP));
            let Some(screen) = self.screen else { break };
            if at.x < 1.0 || at.y < 1.0 || at.x >= screen.width - 1.0 || at.y >= screen.height - 1.0
            {
                at = here;
                self.button_up(button)?;
                self.move_pointer(at)?;
                self.button_down(button)?;
                continue;
            }
            if let Err(failure) = self.move_pointer(at) {
                outcome = Err(failure);
                break;
            }
            moved += step;
        }
        self.button_up(button)?;
        for key in modifiers.iter().rev() {
            self.key_up(key)?;
        }
        outcome.map(|()| pressed)
    }

    /// The view once the camera has come to rest: unchanged over a couple of frames.
    fn camera_settled(&mut self) -> Result<ProbeView, Failure> {
        let mut last: Option<(Radians, Radians)> = None;
        self.wait_for("the camera to settle", BUDGET, move |snapshot| {
            let view = snapshot.view.as_ref()?;
            let now = (view.yaw, view.pitch);
            let settled = last == Some(now);
            last = Some(now);
            settled.then_some(*view)
        })
    }

    /// Where the ground nearest the player shows uncovered in the window: under the player, or
    /// a few metres off where a window covers them.
    fn open_ground(&mut self) -> Result<Pos<WindowPx>, Failure> {
        let snapshot = self.snapshot()?;
        let me = snapshot
            .me
            .as_ref()
            .ok_or_else(|| Failure::Harness("the player is not in the world".to_owned()))?
            .at;
        OPEN_GROUND_OFFSETS
            .iter()
            .find_map(|&(dx, dy)| clickable_point(&snapshot, Pos::new(me.x + dx, me.y + dy)))
            .ok_or_else(|| Failure::Harness("no open ground shows near the player".to_owned()))
    }

    /// How far round, in degrees on screen, the ground at `to` lies from straight ahead of the
    /// player, to the right when positive.
    fn ahead_of(&mut self, to: Pos<Meters>) -> Result<f32, Failure> {
        let snapshot = self.snapshot()?;
        let me = snapshot
            .me
            .as_ref()
            .ok_or_else(|| Failure::Harness("the player is not in the world".to_owned()))?
            .at;
        let view = snapshot
            .view
            .as_ref()
            .ok_or_else(|| Failure::Harness("no view to face with".to_owned()))?;
        let height = |at| {
            snapshot
                .terrain
                .as_ref()
                .map_or(Meters(0.0), |terrain| terrain.height_at(at))
        };
        let from = view.window_point(me, height(me));
        let ahead = view.window_point(to, height(to));
        Ok((ahead.x - from.x).atan2(from.y - ahead.y).to_degrees())
    }

    /// The button and the modifiers the action is bound to, as a drag, a click or a press.
    fn mouse_binding(&self, action: InputAction) -> Result<(Button, KeyModifiers), Failure> {
        self.inputs
            .bindings(action)
            .iter()
            .find_map(|binding| match binding {
                InputBinding::Drag(drag) => Some((button_of(drag.button), drag.modifiers)),
                InputBinding::Click(click) => Some((button_of(click.button), click.modifiers)),
                InputBinding::Mouse { button, modifiers } => Some((button_of(*button), *modifiers)),
                _ => None,
            })
            .ok_or_else(|| Failure::Harness(format!("no mouse button is bound to {action:?}")))
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
        let mut landed = None;
        self.hold(&gesture.modifiers, |driver| {
            landed = Some(driver.key(&keysym(&format!("{:?}", gesture.key)))?);
            Ok(())
        })?;
        landed.ok_or_else(|| Failure::Harness("the key never landed".to_owned()))
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
const CLICK_APART_MS: &str = "40";
const WHEEL_APART_MS: &str = "10";
const TYPE_APART_MS: &str = "30";
/// How many times the windows are scrolled down looking for an element, and by how much.
const SCROLL_TRIES: u32 = 8;
const SCROLL_LINES: i32 = 3;
/// How many dismissals may be needed to close everything open over the world.
const CLOSE_TRIES: u32 = 12;
/// Steps of the pointer a drag goes in.
const DRAG_STEPS: u32 = 8;
/// Window pixels a window is dragged by, across and down.
const WINDOW_DRAG: f32 = 120.0;
/// Turns the camera may take to come to face a point, and how far off it may rest, in degrees.
const FACE_TRIES: u32 = 12;
const FACE_WITHIN: f32 = 4.0;
/// How little a player may move and still count as standing where it was.
const STILL_WITHIN: Meters = Meters(0.05);
/// How many frames a player must have rested for, so a step's end isn't taken for rest.
const REST_FRAMES: u32 = 12;
/// How long between asking the server again to stage what it couldn't yet.
const STAGE_RETRY: Duration = Duration::from_millis(500);
/// How long a player has to set off on a walk before it is asked for again.
const SET_OFF_WITHIN: Duration = Duration::from_secs(3);
/// Metres off the player, nearest first, where open ground is looked for when a window covers
/// them.
const OPEN_GROUND_OFFSETS: [(f32, f32); 13] = [
    (0.0, 0.0),
    (2.0, 0.0),
    (-2.0, 0.0),
    (0.0, 2.0),
    (0.0, -2.0),
    (4.0, 0.0),
    (-4.0, 0.0),
    (0.0, 4.0),
    (0.0, -4.0),
    (4.0, 4.0),
    (-4.0, 4.0),
    (4.0, -4.0),
    (-4.0, -4.0),
];
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

    fn scaled(self, by: f32) -> Nudge {
        Nudge {
            dx: WindowPx(self.dx.0 * by),
            dy: WindowPx(self.dy.0 * by),
        }
    }
}

fn nudged(at: Pos<WindowPx>, by: Nudge) -> Pos<WindowPx> {
    Pos::new(at.x + by.dx.0, at.y + by.dy.0)
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

fn shown_element(snapshot: &ProbeSnapshot, target: &UiTarget) -> Option<ProbeUiElement> {
    snapshot
        .ui
        .iter()
        .find(|element| !element.in_world && !element.click_through && element_is(element, target))
        .cloned()
}

fn element_is(element: &ProbeUiElement, target: &UiTarget) -> bool {
    match target {
        UiTarget::Text(text) => element.text.as_deref() == Some(text.as_str()),
        UiTarget::Field => element.editable,
    }
}

/// What the interface showed at `point` in `snapshot`: every element there, by what it reads, and
/// whether it takes the pointer there.
fn under_point(snapshot: &ProbeSnapshot, point: Pos<WindowPx>) -> String {
    let elements: Vec<String> = snapshot
        .ui
        .iter()
        .filter(|element| element.rect.contains(point))
        .map(|element| {
            let reads = element
                .text
                .as_deref()
                .or(element.image.as_deref())
                .unwrap_or("nothing");
            let kind = if element.in_world {
                "in the world"
            } else if element.click_through {
                "clicked through"
            } else {
                "taking clicks"
            };
            format!("{reads:?} {kind}")
        })
        .collect();
    let covered = snapshot.covered.iter().any(|cover| cover.contains(point));
    format!(
        "{} with the pointer seen at {:?}, {}",
        if elements.is_empty() {
            "no element".to_owned()
        } else {
            elements.join(", ")
        },
        snapshot.input.cursor.map(|cursor| (cursor.x, cursor.y)),
        if covered {
            "the interface taking the pointer there"
        } else {
            "the world taking the pointer there"
        }
    )
}

/// Why the ground at `at` can't be pointed at: off the window, or under a window of the
/// interface, named by the first thing it reads.
fn hidden_by(snapshot: &ProbeSnapshot, at: Pos<Meters>) -> String {
    let Some(view) = &snapshot.view else {
        return "unseen: there is no view".to_owned();
    };
    let height = snapshot
        .terrain
        .as_ref()
        .map_or(Meters(0.0), |terrain| terrain.height_at(at));
    let point = view.window_point(at, height).round();
    let covers: Vec<String> = snapshot
        .covered
        .iter()
        .filter(|cover| cover.contains(point))
        .map(|cover| {
            let reads = snapshot
                .ui
                .iter()
                .filter(|element| cover.contains(element.rect.center()))
                .filter_map(|element| element.text.as_deref())
                .next()
                .unwrap_or("nothing");
            format!(
                "a window reading {reads:?} at {:?}",
                (cover.min().x, cover.min().y, cover.width(), cover.height())
            )
        })
        .collect();
    if covers.is_empty() {
        format!("out of view at {:?}", (point.x, point.y))
    } else {
        format!("under {}", covers.join(", "))
    }
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
