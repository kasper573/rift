use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use serde::Deserialize;
use tiled::{Frame, PropertyValue, TileId};

use crate::assets;
use crate::math::{Millis, PlaybackRate, Pos, Rect, Seconds, Size, Tiles, WorldPx};
use crate::table::Content;

#[derive(Clone, PartialEq, Eq, Hash, Debug, Deserialize)]
pub struct SfxId(pub String);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timing {
    pub duration: Seconds,
    pub apex: Seconds,
}

pub struct ActorModel {
    name: String,
    sheet: String,
    frame: Size<WorldPx>,
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

    pub fn hitbox(&self) -> Size<Tiles> {
        self.hitbox
    }

    pub fn sheet(&self) -> &str {
        &self.sheet
    }

    pub fn frame(
        &self,
        action: &str,
        dir: u8,
        t: Seconds,
        attack_speed: PlaybackRate,
    ) -> Rect<WorldPx> {
        let strip = self.strip(action, dir);
        let elapsed = Millis((t.0 * 1000.0 * rate(action, attack_speed).0).max(0.0));
        let total = total_ms(strip);
        let position = if action == DEATH {
            Millis(elapsed.0.min(total.0 - 1.0))
        } else {
            Millis(elapsed.0 % total.0)
        };
        let mut cursor = Millis(0.0);
        for frame in strip {
            cursor += Millis(frame.duration as f32);
            if position < cursor {
                return self.region(frame.tile_id);
            }
        }
        self.region(strip[strip.len() - 1].tile_id)
    }

    pub fn timing(&self, action: &str, dir: u8) -> Timing {
        let strip = self.strip(action, dir);
        let mut apex = Millis(0.0);
        let mut cursor = Millis(0.0);
        for frame in strip {
            if self.apexes.contains(&frame.tile_id) {
                apex = cursor;
            }
            cursor += Millis(frame.duration as f32);
        }
        Timing {
            duration: cursor.seconds(),
            apex: apex.seconds(),
        }
    }

    pub fn cues(
        &self,
        action: &str,
        dir: u8,
        prev: Seconds,
        now: Seconds,
        attack_speed: PlaybackRate,
    ) -> (Vec<&SfxId>, bool) {
        let strip = self
            .strips
            .get(action)
            .map(|dirs| dirs[dir_slot(dir)].as_slice())
            .unwrap_or_default();
        let rate = rate(action, attack_speed);
        let authored = |t: Seconds| Millis(t.0 * 1000.0 * rate.0);
        let total = total_ms(strip);
        let once = action == DEATH;
        let (mut sfx, mut stepped) = (Vec::new(), false);
        let mut cursor = Millis(0.0);
        for frame in strip {
            if crossed(once, total, cursor, authored(prev), authored(now)) {
                sfx.extend(self.sounds.get(&frame.tile_id));
                stepped |= self.steps.contains(&frame.tile_id);
            }
            cursor += Millis(frame.duration as f32);
        }
        (sfx, stepped)
    }

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

    fn region(&self, tile: TileId) -> Rect<WorldPx> {
        Rect::new(
            Pos::new(
                (tile % self.columns) as f32 * self.frame.width,
                (tile / self.columns) as f32 * self.frame.height,
            ),
            self.frame,
        )
    }
}

impl Content for ActorModel {
    fn table() -> &'static [ActorModel] {
        models()
    }
    fn id(&self) -> &str {
        &self.name
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

fn rate(action: &str, attack_speed: PlaybackRate) -> PlaybackRate {
    if action == ATTACK {
        PlaybackRate(attack_speed.0.max(0.01))
    } else {
        PlaybackRate(1.0)
    }
}

fn total_ms(strip: &[Frame]) -> Millis {
    Millis(strip.iter().map(|frame| frame.duration as f32).sum())
}

fn crossed(once: bool, total: Millis, at: Millis, prev: Millis, now: Millis) -> bool {
    let count = |t: Millis| {
        if t < at {
            0
        } else if once {
            1
        } else {
            ((t.0 - at.0) / total.0) as i64 + 1
        }
    };
    count(now) > count(prev)
}
