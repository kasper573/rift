//! A player's hands on an app the bench drives: the pointer moved, buttons and keys pressed, the
//! wheel turned and text typed through xdotool on the bench's desktop, each confirmed by the
//! input counters the app's probe reports before the next.

use std::process::Command;
use std::time::{Duration, Instant};

use engine::core::math::Pos;
use engine::core::units::WindowPx;
use engine::systems::input::map::KeyModifiers;
use engine::systems::probe::{ProbeInput, ProbeInputSeen};

use super::scenario::Button;

/// Why a scenario could not be played through.
#[derive(Clone, Debug)]
pub enum Failure {
    /// An input never confirmed, a target never found, a wait run out, or the probe's ring of
    /// frames wrapped before it was read.
    Harness(String),
    /// The app, or the server it plays on, died.
    Crashed(String),
}

/// An app the bench drives through its window, answering through its probe.
pub trait Driven {
    type Snapshot: Clone;

    /// What the app shows in a frame after asking; keeps the input, pointer and frame it tells.
    fn snapshot(&mut self) -> Result<Self::Snapshot, Failure>;
    fn input(snapshot: &Self::Snapshot) -> &ProbeInput;
    fn frame_of(snapshot: &Self::Snapshot) -> u32;
    /// The input the latest snapshot counted.
    fn seen(&self) -> ProbeInputSeen;
    /// Where the latest snapshot had the pointer.
    fn cursor(&self) -> Option<Pos<WindowPx>>;
    /// The latest frame seen.
    fn frame(&self) -> u32;
    /// The X display the app's window is on.
    fn display(&self) -> &str;
    /// Notes a step the app was put through in the steps log.
    fn note(&mut self, step: &str);
}

/// Asks for snapshots until one is `found`, giving up once `budget` has passed and the app has
/// drawn as many frames as a fast machine would have in it.
pub fn wait_for<D: Driven, T>(
    driven: &mut D,
    what: &str,
    budget: Duration,
    mut found: impl FnMut(&D::Snapshot) -> Option<T>,
) -> Result<T, Failure> {
    let since = Instant::now();
    let first = driven.frame();
    loop {
        let snapshot = driven.snapshot()?;
        if let Some(done) = found(&snapshot) {
            return Ok(done);
        }
        if since.elapsed() >= budget && driven.frame().saturating_sub(first) >= FAST_FRAMES {
            return Err(Failure::Harness(format!("gave up waiting for {what}")));
        }
    }
}

/// Waits for the app to draw `frames` more frames: what it shows once it has.
pub fn frames<D: Driven>(driven: &mut D, frames: u32) -> Result<D::Snapshot, Failure> {
    let until = driven.frame() + frames;
    wait_for(driven, "the frames to draw", BUDGET, move |snapshot| {
        (D::frame_of(snapshot) >= until).then(|| snapshot.clone())
    })
}

pub fn move_pointer<D: Driven>(driven: &mut D, to: Pos<WindowPx>) -> Result<D::Snapshot, Failure> {
    let to = Pos::new(to.x.round(), to.y.round());
    let moves = mouse_moves(driven.cursor(), &[to], None);
    xdotool(driven, &moves)?;
    let seen = wait_for(driven, "the pointer to arrive", BUDGET, move |snapshot| {
        let cursor = D::input(snapshot).cursor?;
        (cursor.distance_to(to) <= 1.0).then(|| snapshot.clone())
    })?;
    driven.note(&format!("pointer at {:?}", (to.x, to.y)));
    Ok(seen)
}

/// Through every point as fast as a flick of the hand, coming to rest on the last.
pub fn sweep<D: Driven>(driven: &mut D, through: &[Pos<WindowPx>]) -> Result<D::Snapshot, Failure> {
    let Some(last) = through.last() else {
        return driven.snapshot();
    };
    let to = Pos::new(last.x.round(), last.y.round());
    let through: Vec<Pos<WindowPx>> = through
        .iter()
        .map(|at| Pos::new(at.x.round(), at.y.round()))
        .collect();
    let moves = mouse_moves(driven.cursor(), &through, Some(SWEEP_STEP));
    xdotool(driven, &moves)?;
    let seen = wait_for(
        driven,
        "the pointer to come to rest",
        BUDGET,
        move |snapshot| {
            let cursor = D::input(snapshot).cursor?;
            (cursor.distance_to(to) <= 1.0).then(|| snapshot.clone())
        },
    )?;
    driven.note(&format!("pointer swept to {:?}", (to.x, to.y)));
    Ok(seen)
}

pub fn click<D: Driven>(driven: &mut D, button: Button, times: u8) -> Result<D::Snapshot, Failure> {
    let before = driven.seen();
    xdotool(
        driven,
        &[
            "click".to_owned(),
            "--repeat".to_owned(),
            times.to_string(),
            "--delay".to_owned(),
            CLICK_APART_MS.to_owned(),
            button.x_number().to_string(),
        ],
    )?;
    let landed = wait_for(driven, "the click to land", BUDGET, move |snapshot| {
        let seen = &D::input(snapshot).seen;
        (seen.mouse_presses >= before.mouse_presses + u32::from(times)
            && seen.mouse_releases >= before.mouse_releases + u32::from(times))
        .then(|| snapshot.clone())
    })?;
    driven.note(&format!("click {times}× button {}", button.x_number()));
    Ok(landed)
}

pub fn button_down<D: Driven>(driven: &mut D, button: Button) -> Result<D::Snapshot, Failure> {
    let before = driven.seen();
    xdotool(
        driven,
        &["mousedown".to_owned(), button.x_number().to_string()],
    )?;
    wait_for(driven, "the button to go down", BUDGET, move |snapshot| {
        (D::input(snapshot).seen.mouse_presses > before.mouse_presses).then(|| snapshot.clone())
    })
}

pub fn button_up<D: Driven>(driven: &mut D, button: Button) -> Result<D::Snapshot, Failure> {
    let before = driven.seen();
    xdotool(
        driven,
        &["mouseup".to_owned(), button.x_number().to_string()],
    )?;
    wait_for(driven, "the button to come up", BUDGET, move |snapshot| {
        (D::input(snapshot).seen.mouse_releases > before.mouse_releases).then(|| snapshot.clone())
    })
}

/// X turns the wheel by clicking buttons 4 (up) and 5 (down), a line per click.
pub fn wheel<D: Driven>(driven: &mut D, lines: i32) -> Result<D::Snapshot, Failure> {
    let before = driven.seen();
    let button = if lines < 0 { "4" } else { "5" };
    let turns = lines.unsigned_abs();
    xdotool(
        driven,
        &[
            "click".to_owned(),
            "--repeat".to_owned(),
            turns.to_string(),
            "--delay".to_owned(),
            WHEEL_APART_MS.to_owned(),
            button.to_owned(),
        ],
    )?;
    wait_for(driven, "the wheel to turn", BUDGET, move |snapshot| {
        (D::input(snapshot).seen.wheel_turns >= before.wheel_turns + turns).then_some(())
    })?;
    // winit reports a synthetic wheel button's release as a line as it does its press, so a
    // click scrolls twice, the second time a frame or two after the first is seen.
    let settled = frames(driven, WHEEL_SETTLE)?;
    driven.note(&format!("wheel {lines} lines"));
    Ok(settled)
}

pub fn key<D: Driven>(driven: &mut D, keysym: &str) -> Result<D::Snapshot, Failure> {
    let before = driven.seen();
    xdotool(driven, &["key".to_owned(), keysym.to_owned()])?;
    let landed = wait_for(driven, "the key press to land", BUDGET, move |snapshot| {
        (D::input(snapshot).seen.key_presses > before.key_presses).then(|| snapshot.clone())
    })?;
    driven.note(&format!("key {keysym}"));
    Ok(landed)
}

pub fn key_down<D: Driven>(driven: &mut D, keysym: &str) -> Result<D::Snapshot, Failure> {
    let before = driven.seen();
    xdotool(driven, &["keydown".to_owned(), keysym.to_owned()])?;
    wait_for(driven, "the key to go down", BUDGET, move |snapshot| {
        (D::input(snapshot).seen.key_presses > before.key_presses).then(|| snapshot.clone())
    })
}

/// Releases count nowhere, but X delivers them before anything sent after.
pub fn key_up<D: Driven>(driven: &mut D, keysym: &str) -> Result<(), Failure> {
    xdotool(driven, &["keyup".to_owned(), keysym.to_owned()])
}

/// Typed a key at a time, as a player types.
pub fn type_text<D: Driven>(driven: &mut D, text: &str) -> Result<(), Failure> {
    let before = driven.seen();
    let typed = text.chars().count() as u32;
    xdotool(
        driven,
        &[
            "type".to_owned(),
            "--delay".to_owned(),
            TYPE_APART_MS.to_owned(),
            "--".to_owned(),
            text.to_owned(),
        ],
    )?;
    wait_for(driven, "the typing to land", BUDGET, move |snapshot| {
        (D::input(snapshot).seen.key_presses >= before.key_presses + typed).then_some(())
    })?;
    driven.note(&format!("typed {text:?}"));
    Ok(())
}

/// Holds `button` with `held` from `from` and moves the pointer to `to` a step at a time.
pub fn drag<D: Driven>(
    driven: &mut D,
    button: Button,
    held: &KeyModifiers,
    from: Pos<WindowPx>,
    to: Pos<WindowPx>,
) -> Result<(), Failure> {
    move_pointer(driven, from)?;
    hold(driven, held, |driven| {
        button_down(driven, button)?;
        for step in 1..=DRAG_STEPS {
            move_pointer(driven, from.lerp(to, step as f32 / DRAG_STEPS as f32))?;
        }
        button_up(driven, button).map(drop)
    })
}

/// Does `with` while the modifier keys of `held` are down.
pub fn hold<D: Driven>(
    driven: &mut D,
    held: &KeyModifiers,
    with: impl FnOnce(&mut D) -> Result<(), Failure>,
) -> Result<(), Failure> {
    let modifiers = modifier_keys(held);
    for key in &modifiers {
        key_down(driven, key)?;
    }
    let outcome = with(driven);
    for key in modifiers.iter().rev() {
        key_up(driven, key)?;
    }
    outcome
}

/// The X keysyms of the modifier keys `held` holds down.
pub fn modifier_keys(held: &KeyModifiers) -> Vec<&'static str> {
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
pub fn keysym(key: &str) -> String {
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

/// How long a wait lasts, in time; a fast machine must also have drawn the frames it would in
/// that time, so a slow machine gets as many frames as a fast one.
pub const BUDGET: Duration = Duration::from_secs(20);
pub const FAST_FRAMES: u32 = 20 * 60;

/// The xdotool commands that move the pointer from `from` through each of `points` as a mouse
/// does, with a `pause` after each when given. X reports the mouse moving only for a move by an
/// offset, and lands that move where its pointer acceleration takes it, so each goes by its offset
/// and is then set on its point.
fn mouse_moves(
    from: Option<Pos<WindowPx>>,
    points: &[Pos<WindowPx>],
    pause: Option<&str>,
) -> Vec<String> {
    let mut last = from;
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

fn xdotool<D: Driven>(driven: &D, args: &[String]) -> Result<(), Failure> {
    let run = Command::new("xdotool")
        .env("DISPLAY", driven.display())
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

const CLICK_APART_MS: &str = "40";
const WHEEL_APART_MS: &str = "10";
const TYPE_APART_MS: &str = "30";
/// Frames a turn of the wheel takes to settle.
const WHEEL_SETTLE: u32 = 2;
/// Steps of the pointer a drag goes in.
const DRAG_STEPS: u32 = 8;
/// Seconds between the moves of a sweep: a mouse reports a few hundred times a second.
const SWEEP_STEP: &str = "0.004";
