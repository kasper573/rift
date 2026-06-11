//! The embedded actor registry: every `assets/actors/<name>.tsx` is a Tiled tileset over the
//! model's sheet (`<name>.png`), sorted by name — the index is the wire id. A sheet holds one
//! row of frame-sized cells per action and direction; the strip for an action+direction is the
//! tile animation on the row's first cell (its `action`/`dir` properties name it), and frame
//! metadata rides on the member tiles' properties (`sfx`, `step`, `apex`), the click hitbox on
//! the tileset's.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use serde::{Deserialize, Deserializer, Serialize};
use tiled::{Frame, PropertyValue, TileId};

use crate::assets;
use crate::math::{Pixels, Pos, Rect, Size, Tiles};

/// An actor model's index in [`models`]; content tables reference models by name via
/// [`model_by_name`].
#[derive(
    Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default,
)]
pub struct ActorModelId(pub u16);

pub fn model_by_name<'de, D: Deserializer<'de>>(deserializer: D) -> Result<ActorModelId, D::Error> {
    let name = String::deserialize(deserializer)?;
    model_index(&name)
        .ok_or_else(|| serde::de::Error::custom(format!("unknown actor model '{name}'")))
}

#[derive(Clone, PartialEq, Eq, Hash, Debug, Deserialize)]
pub struct SfxId(pub String);

/// One animation run's place in time; the apex of an action without one is 0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timing {
    pub duration: f32,
    pub apex: f32,
}

/// An 8-direction actor model. Frames advance at their authored durations (scaled by attack
/// speed for "attack"); actions loop, except "death" which plays once and holds its last frame.
/// Unknown actions fall back to idle. Audio cues fire as their frame is entered.
pub struct ActorModel {
    name: String,
    sheet: String,
    frame: Size<Pixels>,
    columns: u32,
    hitbox: Size<Tiles>,
    strips: HashMap<String, [Vec<Frame>; 8]>,
    sounds: HashMap<TileId, SfxId>,
    steps: HashSet<TileId>,
    apexes: HashSet<TileId>,
}

impl ActorModel {
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The click target in tiles: a box centered on x with its bottom at the feet line.
    pub fn hitbox(&self) -> Size<Tiles> {
        self.hitbox
    }

    /// The model's sheet as a root-relative asset path; [`ActorModel::frame`] regions index into it.
    pub fn sheet(&self) -> &str {
        &self.sheet
    }

    /// The frame to draw `t` seconds into `action`, as a pixel region of [`ActorModel::sheet`].
    pub fn frame(&self, action: &str, dir: u8, t: f32, attack_speed: f32) -> Rect<Pixels> {
        let strip = self.strip(action, dir);
        let elapsed = (t * 1000.0 * rate(action, attack_speed)).max(0.0);
        let total = total_ms(strip);
        let position = if action == DEATH {
            elapsed.min(total - 1.0)
        } else {
            elapsed % total
        };
        let mut cursor = 0.0;
        for frame in strip {
            cursor += frame.duration as f32;
            if position < cursor {
                return self.region(frame.tile_id);
            }
        }
        self.region(strip[strip.len() - 1].tile_id)
    }

    /// The timing of `action` facing `dir` at the authored pace: total run length and the moment
    /// of its apex frame, both in seconds.
    pub fn timing(&self, action: &str, dir: u8) -> Timing {
        let strip = self.strip(action, dir);
        let mut apex = 0.0;
        let mut cursor = 0.0;
        for frame in strip {
            if self.apexes.contains(&frame.tile_id) {
                apex = cursor;
            }
            cursor += frame.duration as f32;
        }
        Timing {
            duration: cursor / 1000.0,
            apex: apex / 1000.0,
        }
    }

    /// The sound cues whose frames were entered as time-into-`action` advanced from `prev` to
    /// `now` seconds (pass `prev < 0.0` for "the action just started"), and whether a step frame
    /// was crossed. A looping action refires its cues each cycle, "death" fires once and never
    /// again while it holds the last frame, a single call fires each cue at most once so a long
    /// pause can't replay it, and unknown actions are silent.
    pub fn cues(
        &self,
        action: &str,
        dir: u8,
        prev: f32,
        now: f32,
        attack_speed: f32,
    ) -> (Vec<&SfxId>, bool) {
        let strip = self
            .strips
            .get(action)
            .map(|dirs| dirs[dir_slot(dir)].as_slice())
            .unwrap_or_default();
        let rate = rate(action, attack_speed);
        let total = total_ms(strip);
        let once = action == DEATH;
        let (mut sfx, mut stepped) = (Vec::new(), false);
        let mut cursor = 0.0;
        for frame in strip {
            if crossed(
                once,
                total,
                cursor,
                prev * 1000.0 * rate,
                now * 1000.0 * rate,
            ) {
                sfx.extend(self.sounds.get(&frame.tile_id));
                stepped |= self.steps.contains(&frame.tile_id);
            }
            cursor += frame.duration as f32;
        }
        (sfx, stepped)
    }

    /// Every cue id this model references.
    pub fn sfx_ids(&self) -> impl Iterator<Item = &SfxId> {
        self.sounds.values()
    }

    fn strip(&self, action: &str, dir: u8) -> &[Frame] {
        let spec = self.strips.get(action).unwrap_or_else(|| {
            self.strips
                .get(IDLE)
                .expect("validated at load: every actor model declares idle")
        });
        &spec[dir_slot(dir)]
    }

    fn region(&self, tile: TileId) -> Rect<Pixels> {
        Rect::new(
            Pos::new(
                (tile % self.columns) as f32 * self.frame.width,
                (tile / self.columns) as f32 * self.frame.height,
            ),
            self.frame,
        )
    }
}

pub fn models() -> &'static [ActorModel] {
    static MODELS: OnceLock<Vec<ActorModel>> = OnceLock::new();
    MODELS.get_or_init(|| {
        let mut all: Vec<ActorModel> = assets::list(assets::ACTORS)
            .iter()
            .filter(|path| path.ends_with(".tsx"))
            .map(|path| load(assets::stem(path)))
            .collect();
        all.sort_unstable_by(|a, b| a.name.cmp(&b.name));
        all
    })
}

pub fn model(id: ActorModelId) -> &'static ActorModel {
    &models()[id.0 as usize]
}

pub fn model_index(name: &str) -> Option<ActorModelId> {
    models()
        .iter()
        .position(|model| model.name() == name)
        .map(|index| ActorModelId(index as u16))
}

const IDLE: &str = "idle";
const ATTACK: &str = "attack";
const DEATH: &str = "death";

fn load(name: &str) -> ActorModel {
    let tileset = tiled::Loader::with_reader(assets::tiled_reader)
        .load_tsx_tileset(format!("{}/{name}.tsx", assets::ACTORS))
        .unwrap_or_else(|error| panic!("actor model {name}: {error}"));
    let source = tileset
        .image
        .as_ref()
        .and_then(|image| image.source.file_name()?.to_str())
        .unwrap_or_else(|| panic!("actor model {name} declares no sheet image"));
    let sheet = assets::find(assets::ACTORS, source)
        .unwrap_or_else(|| panic!("actor model {name} has no sheet {source}"));

    let mut strips: HashMap<String, [Vec<Frame>; 8]> = HashMap::new();
    let mut sounds = HashMap::new();
    let mut steps = HashSet::new();
    let mut apexes = HashSet::new();
    for (id, tile) in tileset.tiles() {
        if let Some(PropertyValue::StringValue(sfx)) = tile.properties.get("sfx") {
            sounds.insert(id, SfxId(sfx.clone()));
        }
        if let Some(PropertyValue::BoolValue(true)) = tile.properties.get("step") {
            steps.insert(id);
        }
        if let Some(PropertyValue::BoolValue(true)) = tile.properties.get("apex") {
            apexes.insert(id);
        }
        if let Some(PropertyValue::StringValue(action)) = tile.properties.get("action") {
            let dir = match tile.properties.get("dir") {
                Some(PropertyValue::IntValue(dir)) if (0..8).contains(dir) => *dir as usize,
                _ => panic!("actor model {name}: '{action}' tile {id} needs a dir in 0..8"),
            };
            let strip = tile
                .animation
                .clone()
                .filter(|frames| !frames.is_empty())
                .unwrap_or_else(|| panic!("actor model {name}: '{action}' dir {dir} is empty"));
            strips.entry(action.clone()).or_default()[dir] = strip;
        }
    }
    for (action, dirs) in &strips {
        if dirs.iter().any(Vec::is_empty) {
            panic!("actor model {name}: action '{action}' is missing a direction strip");
        }
    }
    if !strips.contains_key(IDLE) {
        panic!("actor model {name} must declare an idle action");
    }

    let dimension = |key: &str| match tileset.properties.get(key) {
        Some(PropertyValue::FloatValue(value)) => *value,
        _ => panic!("actor model {name} needs a float '{key}' tileset property"),
    };
    ActorModel {
        name: name.to_owned(),
        sheet,
        frame: Size::new(tileset.tile_width as f32, tileset.tile_height as f32),
        columns: tileset.columns.max(1),
        hitbox: Size::new(dimension("hitbox_width"), dimension("hitbox_height")),
        strips,
        sounds,
        steps,
        apexes,
    }
}

fn dir_slot(dir: u8) -> usize {
    if dir > 7 { 0 } else { dir as usize }
}

fn rate(action: &str, attack_speed: f32) -> f32 {
    if action == ATTACK {
        attack_speed.max(0.01)
    } else {
        1.0
    }
}

fn total_ms(strip: &[Frame]) -> f32 {
    strip.iter().map(|frame| frame.duration as f32).sum()
}

// Whether the cue moment `at` ms into the strip is crossed as scaled time-into-action advances
// from `prev` to `now` ms; shared by sound and step cues so every cue fires exactly as its frame
// is entered. Looping actions refire each cycle, `once` (death) fires a single time ever.
fn crossed(once: bool, total: f32, at: f32, prev: f32, now: f32) -> bool {
    let count = |t: f32| {
        if t < at {
            0
        } else if once {
            1
        } else {
            ((t - at) / total) as i64 + 1
        }
    };
    count(now) > count(prev)
}
