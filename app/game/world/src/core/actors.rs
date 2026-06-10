//! The embedded actor registry: every `assets/actors/<name>.json` describes one pre-baked
//! model, sorted by name — the index is the wire id. A model's sheet (`<name>.png`) holds one
//! row of frame-sized cells per action and direction, every direction baked in.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::{Deserialize, Deserializer, Serialize};

use crate::core::assets;
use crate::core::math::{Pixels, Pos, Rect, Size, Tiles};
use crate::core::protocol::Hitbox;

/// An actor model's index in [`models`]; content tables reference models by name via
/// [`model_by_name`].
#[derive(
    Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default,
)]
pub struct ActorModelId(pub u16);

/// Deserializes an [`ActorModelId`] from a content table's model name.
pub fn model_by_name<'de, D: Deserializer<'de>>(deserializer: D) -> Result<ActorModelId, D::Error> {
    let name = String::deserialize(deserializer)?;
    model_index(&name)
        .ok_or_else(|| serde::de::Error::custom(format!("unknown actor model '{name}'")))
}

#[derive(Clone, PartialEq, Eq, Hash, Debug, Deserialize)]
pub struct SfxId(pub String);

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Sfx {
    pub id: SfxId,
    pub frame: u32,
}

/// One animation run's place in time; the apex of an action without one is 0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timing {
    pub duration: f32,
    pub apex: f32,
}

/// An 8-direction actor model over a pre-baked sheet. Frames advance at 100ms (scaled by attack
/// speed for "attack"); actions loop, except "death" which plays once and holds its last frame.
/// Unknown actions fall back to idle. Audio cues fire as their frame is entered.
pub struct ActorModel {
    name: String,
    sheet: &'static [u8],
    frame: Size<Pixels>,
    hitbox: Size<Tiles>,
    actions: BTreeMap<String, Action>,
}

impl ActorModel {
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The click target in tiles: a box centered on x with its bottom at the feet line.
    pub fn hitbox(&self) -> Size<Tiles> {
        self.hitbox
    }

    /// The model's sheet as PNG bytes; [`ActorModel::frame`] regions index into it.
    pub fn sheet(&self) -> &'static [u8] {
        self.sheet
    }

    /// The frame to draw `t` seconds into `action`, as a pixel region of [`ActorModel::sheet`].
    pub fn frame(&self, action: &str, dir: u8, t: f32, attack_speed: f32) -> Rect<Pixels> {
        let (row, strip) = self.strip(action, dir);
        let elapsed = (t * 1000.0 / frame_ms(action, attack_speed)).max(0.0) as u32;
        let column = if action == DEATH {
            elapsed.min(strip.frames - 1)
        } else {
            elapsed % strip.frames
        };
        Rect::new(
            Pos::new(
                Pixels(column as f32 * self.frame.x.0),
                Pixels(row as f32 * self.frame.y.0),
            ),
            self.frame,
        )
    }

    /// The timing of `action` facing `dir` at the native 100ms-per-frame pace: total run length
    /// and the moment of its apex frame, both in seconds.
    pub fn timing(&self, action: &str, dir: u8) -> Timing {
        let (_, strip) = self.strip(action, dir);
        Timing {
            duration: strip.frames as f32 * FRAME_MS / 1000.0,
            apex: strip.apex.unwrap_or(0) as f32 * FRAME_MS / 1000.0,
        }
    }

    /// Sound cues whose frame was entered as time-into-`action` advanced from `prev` to `now`
    /// seconds; pass `prev < 0.0` for "the action just started". A looping action refires its
    /// cues each cycle, "death" fires once and never again while it holds the last frame, and a
    /// single call fires each cue at most once so a long pause can't replay it.
    pub fn sfx(
        &self,
        action: &str,
        dir: u8,
        prev: f32,
        now: f32,
        attack_speed: f32,
    ) -> Vec<&SfxId> {
        let Some(spec) = self.actions.get(action) else {
            return Vec::new();
        };
        let strip = &spec.dirs[dir_slot(dir)];
        strip
            .sfx
            .iter()
            .filter(|cue| crossed(action, strip.frames, cue.frame, prev, now, attack_speed))
            .map(|cue| &cue.id)
            .collect()
    }

    /// Whether a step-cue frame was crossed as time-into-`action` advanced from `prev` to `now`
    /// seconds, with [`ActorModel::sfx`]'s timing.
    pub fn steps_crossed(
        &self,
        action: &str,
        dir: u8,
        prev: f32,
        now: f32,
        attack_speed: f32,
    ) -> bool {
        let Some(spec) = self.actions.get(action) else {
            return false;
        };
        let strip = &spec.dirs[dir_slot(dir)];
        strip
            .steps
            .iter()
            .any(|&frame| crossed(action, strip.frames, frame, prev, now, attack_speed))
    }

    /// Every cue id this model references.
    pub fn sfx_ids(&self) -> impl Iterator<Item = &SfxId> {
        self.actions
            .values()
            .flat_map(|spec| spec.dirs.iter())
            .flat_map(|strip| strip.sfx.iter().map(|cue| &cue.id))
    }

    fn strip(&self, action: &str, dir: u8) -> (u32, &Strip) {
        let spec = self.actions.get(action).unwrap_or_else(|| {
            self.actions
                .get(IDLE)
                .expect("validated at load: every actor model declares idle")
        });
        let slot = dir_slot(dir);
        (spec.row + slot as u32, &spec.dirs[slot])
    }
}

pub fn models() -> &'static [ActorModel] {
    static MODELS: OnceLock<Vec<ActorModel>> = OnceLock::new();
    MODELS.get_or_init(|| {
        let mut all: Vec<ActorModel> = assets::dir(assets::ACTORS)
            .filter_map(|(path, bytes)| {
                let name = assets::stem(path);
                if !path.ends_with(".json") {
                    return None;
                }
                let manifest = std::str::from_utf8(bytes)
                    .unwrap_or_else(|_| panic!("actor model {name}'s manifest is not utf-8"));
                Some(load(name, manifest))
            })
            .collect();
        all.sort_unstable_by(|a, b| a.name.cmp(&b.name));
        all
    })
}

pub fn model_index(name: &str) -> Option<ActorModelId> {
    models()
        .iter()
        .position(|model| model.name() == name)
        .map(|index| ActorModelId(index as u16))
}

pub fn model_hitbox(model: ActorModelId) -> Hitbox {
    let size = models()[model.0 as usize].hitbox();
    Hitbox { size }
}

const IDLE: &str = "idle";
const ATTACK: &str = "attack";
const DEATH: &str = "death";
const FRAME_MS: f32 = 100.0;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    frame: Frame,
    hitbox: (f32, f32),
    actions: BTreeMap<String, Action>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Frame {
    w: u32,
    h: u32,
}

/// One action: its base row in the sheet, then one strip per direction on rows `row..row + 8`.
/// The slot index is the `Direction` discriminant.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Action {
    row: u32,
    dirs: [Strip; 8],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Strip {
    frames: u32,
    #[serde(default)]
    apex: Option<u32>,
    #[serde(default)]
    steps: Vec<u32>,
    #[serde(default)]
    sfx: Vec<Sfx>,
}

fn load(name: &str, manifest: &str) -> ActorModel {
    let manifest: Manifest = serde_json::from_str(manifest)
        .unwrap_or_else(|error| panic!("actor model {name}'s manifest: {error}"));
    let sheet = assets::bytes(&format!("{}/{name}.png", assets::ACTORS))
        .unwrap_or_else(|| panic!("actor model {name} has no sheet png"));
    if !manifest.actions.contains_key(IDLE) {
        panic!("actor model {name}'s manifest must declare an idle action");
    }
    for (action, spec) in &manifest.actions {
        for strip in &spec.dirs {
            if strip.frames == 0 {
                panic!("actor model {name}'s action '{action}' has an empty strip");
            }
        }
    }
    ActorModel {
        name: name.to_owned(),
        sheet,
        frame: Size::new(
            Pixels(manifest.frame.w as f32),
            Pixels(manifest.frame.h as f32),
        ),
        hitbox: Size::new(Tiles(manifest.hitbox.0), Tiles(manifest.hitbox.1)),
        actions: manifest.actions,
    }
}

fn dir_slot(dir: u8) -> usize {
    if dir > 7 { 0 } else { dir as usize }
}

fn frame_ms(action: &str, attack_speed: f32) -> f32 {
    if action == ATTACK {
        FRAME_MS / attack_speed.max(0.01)
    } else {
        FRAME_MS
    }
}

// Rust's `/` truncates toward zero; this floors, so cue counts stay correct below frame 0.
fn floor_div(a: i64, b: i64) -> i64 {
    let q = a / b;
    if a % b != 0 && (a < 0) != (b < 0) {
        q - 1
    } else {
        q
    }
}

// Whether absolute `frame` is entered as time-into-`action` advances from `prev` to `now` seconds;
// shared by sound and step cues so every cue fires exactly as its frame is drawn. Fires at most
// once per call (a long pause can't replay it); "death" fires once and never while it holds.
fn crossed(action: &str, frames: u32, frame: u32, prev: f32, now: f32, attack_speed: f32) -> bool {
    let frame_ms = frame_ms(action, attack_speed);
    let index = |t: f32| {
        if t < 0.0 {
            -1
        } else {
            (t * 1000.0 / frame_ms).floor() as i64
        }
    };
    let (from, to) = (index(prev), index(now));
    if to <= from {
        return false;
    }
    let frames = frames as i64;
    let f = frame as i64;
    if action == DEATH {
        from < f && f <= to
    } else {
        floor_div(to - f, frames) > floor_div(from - f, frames)
    }
}
