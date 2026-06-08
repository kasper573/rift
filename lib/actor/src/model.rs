use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::sync::OnceLock;

use image::{Image, Region, decode_png, png_size};
use math::{Direction, Pixels, Pos, Size, Tiles};
use serde::Deserialize;

#[derive(Clone, PartialEq, Eq, Hash, Debug, Deserialize)]
pub struct SfxId(pub String);

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Sfx {
    pub id: SfxId,
    pub frame: u32,
}

/// An 8-direction actor model loaded from a manifest. The manifest maps any on-disk
/// layout — one sheet, file-per-action, file-per-strip — onto the in-memory model by
/// addressing, per action and direction, one horizontal run of frame-sized cells somewhere
/// in some image file:
///
/// ```json
/// { "frame": { "w": 48, "h": 64 },
///   "hitbox": [1, 2],   // click target in tiles, centered on x, bottom at the feet line
///   "actions": { "<action>": {
///     "base": { <strip fields shared by every direction below> },
///     "dir": { "<s|sw|nw|n|ne|se|e|w>": {   // each direction overrides base: {...base, ...dir}
///       "file": "<path resolved by the load fetcher>",
///       "x": 0, "y": 64,  // pixel offset of the run's first frame (default 0,0)
///       "frames": 8,      // run length (default: as many frames as fit the file width)
///       "cues": { "apex": [5], "steps": [0, 4] },  // named frame markers, each a list (default none)
///       "sfx": [{"id":"swing","frame":4}],         // audio cues; fire as their frame is entered
///       "flip": true      // mirror each frame horizontally (e.g. derive west from east)
///     } }
/// } } }
/// ```
///
/// Loading composes everything into one in-memory image per model; absent directions
/// resolve to the nearest declared one, absent actions fall back to idle. Frames advance
/// at 100ms (scaled by attack speed for "attack"); actions loop, except "death" which
/// plays once and holds its last frame. Audio cues fire as their frame is entered; named cues
/// mark frames game logic reads — combat's hit lands at "apex", a footstep at each "steps" frame.
pub struct ActorModel {
    name: String,
    frame: Size<Pixels>,
    hitbox: Size<Tiles>,
    actions: Vec<(String, [Option<Source>; 8])>,
    image: OnceLock<Image>,
}

impl ActorModel {
    /// Parses and validates a manifest, sizing every strip from PNG headers without
    /// decoding any pixels; `fetch` resolves the manifest's file paths. Panics with
    /// context on any inconsistency — actor models load once at startup, fail-fast.
    pub fn load(
        name: &str,
        manifest: &str,
        fetch: impl Fn(&str) -> Option<&'static [u8]>,
    ) -> ActorModel {
        let manifest: Manifest = serde_json::from_str(manifest)
            .unwrap_or_else(|error| panic!("actor model {name}'s manifest: {error}"));
        let frame = Size::new(
            Pixels(manifest.frame.w.get() as f32),
            Pixels(manifest.frame.h.get() as f32),
        );
        let hitbox = Size::new(Tiles(manifest.hitbox.0), Tiles(manifest.hitbox.1));
        if !(hitbox.x.0 > 0.0 && hitbox.y.0 > 0.0) {
            panic!("actor model {name}'s manifest must declare hitbox as [w, h] in positive tiles");
        }
        let mut next_y = 0;
        let mut actions = Vec::new();
        for (action, spec) in &manifest.actions {
            let dir_sources = spec.dir.slots().map(|over| {
                over.map(|over| {
                    let strip = resolve(name, action, &spec.base, over);
                    load_source(name, action, &strip, frame, &fetch, &mut next_y)
                })
            });
            if dir_sources.iter().all(Option::is_none) {
                panic!("actor model {name}'s manifest action '{action}' declares no directions");
            }
            actions.push((action.clone(), dir_sources));
        }
        let model = ActorModel {
            name: name.to_owned(),
            frame,
            hitbox,
            actions,
            image: OnceLock::new(),
        };
        if model.action(IDLE).is_none() {
            panic!("actor model {name}'s manifest must declare an idle action");
        }
        model
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The click target in tiles: a box centered on x with its bottom at the feet line.
    pub fn hitbox(&self) -> Size<Tiles> {
        self.hitbox
    }

    /// The homogenized atlas, composed (decoded, copied, mirrored) on first use.
    pub fn image(&self) -> &Image {
        self.image.get_or_init(|| compose(self))
    }

    /// The frame to draw `t` seconds into `action`, as a region of [`ActorModel::image`].
    pub fn frame(&self, action: &str, dir: u8, t: f32, attack_speed: f32) -> Region {
        let source = self.source(action, dir);

        let frame_ms = if action == ATTACK {
            FRAME_MS / attack_speed.max(0.01)
        } else {
            FRAME_MS
        };
        let elapsed = (t * 1000.0 / frame_ms).max(0.0) as u32;
        let column = if action == DEATH {
            elapsed.min(source.frames - 1)
        } else {
            elapsed % source.frames
        };

        Region::new(
            Pos::new(
                Pixels(column as f32 * self.frame.x.0),
                Pixels(source.y as f32),
            ),
            self.frame,
        )
    }

    /// Sound cues whose frame was entered as time-into-`action` advanced from `prev` to `now`
    /// seconds; pass `prev < 0.0` for "the action just started". Shares [`ActorModel::frame`]'s
    /// timing so a cue fires exactly as its frame is drawn: a looping action refires its cues each
    /// cycle, "death" fires once and never again while it holds the last frame, and a single call
    /// fires each cue at most once so a long pause can't replay it.
    pub fn sfx(
        &self,
        action: &str,
        dir: u8,
        prev: f32,
        now: f32,
        attack_speed: f32,
    ) -> Vec<&SfxId> {
        let Some(dirs) = self.action(action) else {
            return Vec::new();
        };
        let Some(source) = nearest(dirs, dir) else {
            return Vec::new();
        };
        source
            .sfx
            .iter()
            .filter(|cue| crossed(action, source, cue.frame, prev, now, attack_speed))
            .map(|cue| &cue.id)
            .collect()
    }

    /// Whether the named cue's frame was crossed as time-into-`action` advanced from `prev` to
    /// `now` seconds, with the same timing and fallbacks as [`ActorModel::sfx`]. Game logic names
    /// the cue (e.g. "step" for a footfall); the manifest places its frame per direction.
    pub fn cue_crossed(
        &self,
        action: &str,
        dir: u8,
        name: &str,
        prev: f32,
        now: f32,
        attack_speed: f32,
    ) -> bool {
        let Some(dirs) = self.action(action) else {
            return false;
        };
        let Some(source) = nearest(dirs, dir) else {
            return false;
        };
        let Some(frames) = source.cues.get(name) else {
            return false;
        };
        frames
            .iter()
            .any(|&frame| crossed(action, source, frame, prev, now, attack_speed))
    }

    /// Every cue id this model references.
    pub fn sfx_ids(&self) -> impl Iterator<Item = &SfxId> {
        self.actions
            .iter()
            .flat_map(|(_, dirs)| dirs.iter().flatten())
            .flat_map(|source| source.sfx.iter().map(|cue| &cue.id))
    }

    /// The timing of `action` facing `dir` at the native 100ms-per-frame pace, with the
    /// same direction and idle fallbacks as [`ActorModel::frame`]: total run length and the
    /// moment of its apex frame, both in seconds.
    pub fn timing(&self, action: &str, dir: u8) -> Timing {
        let source = self.source(action, dir);
        Timing {
            duration: source.frames as f32 * FRAME_MS / 1000.0,
            apex: source
                .cues
                .get("apex")
                .and_then(|v| v.first().copied())
                .unwrap_or(0) as f32
                * FRAME_MS
                / 1000.0,
        }
    }

    fn source(&self, action: &str, dir: u8) -> &Source {
        let dirs = self
            .action(action)
            .or_else(|| self.action(IDLE))
            .expect("validated at load: every actor model declares idle");
        nearest(dirs, dir).expect("validated at load: every action has a direction")
    }

    fn action(&self, name: &str) -> Option<&[Option<Source>; 8]> {
        self.actions
            .iter()
            .find(|(action, _)| action == name)
            .map(|(_, dirs)| dirs)
    }
}

/// One animation run's place in time; the apex of an undeclared manifest entry is 0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timing {
    pub duration: f32,
    pub apex: f32,
}

const IDLE: &str = "idle";
const ATTACK: &str = "attack";
const DEATH: &str = "death";
const FRAME_MS: f32 = 100.0;

// Ordered so that ties (e.g. E between SE and NE on a 6-direction model) resolve to the
// southern row, which faces the camera.
const CARDINAL_ORDER: [Direction; 8] = [
    Direction::E,
    Direction::SE,
    Direction::S,
    Direction::SW,
    Direction::W,
    Direction::NW,
    Direction::N,
    Direction::NE,
];

// The BTreeMap keeps action order deterministic, so every load composes the same atlas.
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
    w: NonZeroU32,
    h: NonZeroU32,
}

// `base` supplies fields shared by every direction; each `dir` entry overrides them shallowly,
// like `{...base, ...dir}`. `sfx` is action-level, fired regardless of direction.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Action {
    #[serde(default)]
    base: StripFields,
    dir: DirMap,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DirMap {
    s: Option<StripFields>,
    sw: Option<StripFields>,
    nw: Option<StripFields>,
    n: Option<StripFields>,
    ne: Option<StripFields>,
    se: Option<StripFields>,
    e: Option<StripFields>,
    w: Option<StripFields>,
}

impl DirMap {
    // Slot index is the Direction discriminant.
    fn slots(&self) -> [Option<&StripFields>; 8] {
        [
            &self.s, &self.sw, &self.nw, &self.n, &self.ne, &self.se, &self.e, &self.w,
        ]
        .map(Option::as_ref)
    }
}

// Strip fields as written in `base` or a `dir`: all optional, since either side may omit any.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct StripFields {
    file: Option<String>,
    x: Option<u32>,
    y: Option<u32>,
    frames: Option<u32>,
    cues: Option<BTreeMap<String, Vec<u32>>>,
    sfx: Option<Vec<Sfx>>,
    flip: Option<bool>,
}

// One direction's strip after a `dir` entry overrides the action's `base`, field by field.
struct Resolved {
    file: String,
    from: Pos<u32>,
    frames: Option<u32>,
    cues: BTreeMap<String, Vec<u32>>,
    sfx: Vec<Sfx>,
    flip: bool,
}

fn resolve(name: &str, action: &str, base: &StripFields, over: &StripFields) -> Resolved {
    let file = over
        .file
        .clone()
        .or_else(|| base.file.clone())
        .unwrap_or_else(|| {
            panic!(
                "actor model {name}'s manifest action '{action}' has a direction with no 'file' \
                 (set it in base or the direction)"
            )
        });
    Resolved {
        file,
        from: Pos::new(
            over.x.or(base.x).unwrap_or(0),
            over.y.or(base.y).unwrap_or(0),
        ),
        frames: over.frames.or(base.frames),
        cues: over
            .cues
            .clone()
            .or_else(|| base.cues.clone())
            .unwrap_or_default(),
        sfx: over
            .sfx
            .clone()
            .or_else(|| base.sfx.clone())
            .unwrap_or_default(),
        flip: over.flip.or(base.flip).unwrap_or(false),
    }
}

struct Source {
    bytes: &'static [u8],
    flip: bool,
    from: Pos<u32>,
    y: u32,
    frames: u32,
    cues: BTreeMap<String, Vec<u32>>,
    sfx: Vec<Sfx>,
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
// shared by sound cues and named cues so every cue fires exactly as its frame is drawn. Fires at
// most once per call (a long pause can't replay it); "death" fires once and never while it holds.
fn crossed(
    action: &str,
    source: &Source,
    frame: u32,
    prev: f32,
    now: f32,
    attack_speed: f32,
) -> bool {
    let frame_ms = if action == ATTACK {
        FRAME_MS / attack_speed.max(0.01)
    } else {
        FRAME_MS
    };
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
    let frames = source.frames as i64;
    let f = frame as i64;
    if action == DEATH {
        from < f && f <= to
    } else {
        floor_div(to - f, frames) > floor_div(from - f, frames)
    }
}

fn nearest(dirs: &[Option<Source>; 8], dir: u8) -> Option<&Source> {
    if let Some(source) = dirs.get(dir as usize).and_then(Option::as_ref) {
        return Some(source);
    }
    let want = Direction::from_u8(dir).norm_angle();
    let mut best: Option<(&Source, f32)> = None;
    for candidate in CARDINAL_ORDER {
        let Some(source) = dirs[candidate as usize].as_ref() else {
            continue;
        };
        let distance = (want - candidate.norm_angle()).abs();
        if best.is_none_or(|(_, b)| distance < b) {
            best = Some((source, distance));
        }
    }
    best.map(|(source, _)| source)
}

fn load_source(
    name: &str,
    action: &str,
    strip: &Resolved,
    frame: Size<Pixels>,
    fetch: &impl Fn(&str) -> Option<&'static [u8]>,
    next_y: &mut u32,
) -> Source {
    let file = &strip.file;
    let bytes = fetch(file).unwrap_or_else(|| {
        panic!("actor model {name}'s manifest references unknown file '{file}'")
    });
    let png = png_size(bytes)
        .unwrap_or_else(|| panic!("actor model {name}'s manifest: '{file}' is not a PNG"));
    let (width, height) = (png.x.0 as u32, png.y.0 as u32);
    let (frame_w, frame_h) = (frame.x.0 as u32, frame.y.0 as u32);
    let (x, y) = (strip.from.x, strip.from.y);
    let frames = strip
        .frames
        .unwrap_or_else(|| width.saturating_sub(x) / frame_w);
    if frames == 0 || x + frames * frame_w > width || y + frame_h > height {
        panic!(
            "actor model {name}'s manifest action '{action}': {frames} frames of {frame_w}x{frame_h} \
             at ({x}, {y}) exceed '{file}' ({width}x{height})"
        );
    }
    for (cue, list) in &strip.cues {
        for &at in list {
            if at >= frames {
                panic!(
                    "actor model {name}'s manifest action '{action}': cue '{cue}' frame {at} is not \
                     a frame index within its {frames} frames"
                );
            }
        }
    }
    for cue in &strip.sfx {
        if cue.frame >= frames {
            panic!(
                "actor model {name}'s manifest action '{action}': sfx frame {frame} is not a frame \
                 index within its {frames} frames",
                frame = cue.frame
            );
        }
    }
    let source = Source {
        bytes,
        flip: strip.flip,
        from: Pos::new(x, y),
        y: *next_y,
        frames,
        cues: strip.cues.clone(),
        sfx: strip.sfx.clone(),
    };
    *next_y += frame_h;
    source
}

fn compose(model: &ActorModel) -> Image {
    let (frame_w, frame_h) = (model.frame.x.0 as u32, model.frame.y.0 as u32);
    let sources = || {
        model
            .actions
            .iter()
            .flat_map(|(_, dirs)| dirs.iter().flatten())
    };
    let width = sources().map(|s| s.frames * frame_w).max().unwrap_or(0);
    let height = sources().map(|s| s.y + frame_h).max().unwrap_or(0);
    let mut image = Image::new(width, height);
    let mut decoded: Vec<(&[u8], Image)> = Vec::new();
    for source in sources() {
        let file = match decoded
            .iter()
            .position(|(bytes, _)| std::ptr::eq(*bytes, source.bytes))
        {
            Some(hit) => &decoded[hit].1,
            None => {
                decoded.push((source.bytes, decode_png(source.bytes)));
                &decoded.last().expect("just pushed").1
            }
        };
        for y in 0..frame_h {
            for x in 0..source.frames * frame_w {
                let sx = if source.flip {
                    // Mirror within each frame cell so the frame order stays temporal.
                    let frame = x / frame_w;
                    frame * frame_w + (frame_w - 1 - x % frame_w)
                } else {
                    x
                };
                let from = (((source.from.y + y) * file.width + source.from.x + sx) * 4) as usize;
                let to = (((source.y + y) * width + x) * 4) as usize;
                image.rgba[to..to + 4].copy_from_slice(&file.rgba[from..from + 4]);
            }
        }
    }
    image
}
