use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::path::Path;

use tiled::{Frame, PropertyValue, TileId};

use crate::core::assets::AssetRef;
use crate::core::math::{Direction, Pos, Rect, Size, WorldPx};
use crate::core::tiling::Tiles;
use crate::core::time::{Millis, PlaybackRate, Seconds};
use crate::systems::sfx::SfxId;

pub fn load(source: AssetRef) -> ActorModel {
    let name = source.0;
    let bytes = source
        .resolve()
        .unwrap_or_else(|| panic!("actor model {name}: missing asset"))
        .contents();
    let tileset =
        tiled::Loader::with_reader(move |_: &Path| std::io::Result::Ok(Cursor::new(bytes)))
            .load_tsx_tileset(name)
            .unwrap_or_else(|error| panic!("actor model {name}: {error}"));
    let sheet = tileset
        .image
        .as_ref()
        .and_then(|image| image.source.to_str())
        .unwrap_or_else(|| panic!("actor model {name} declares no sheet image"))
        .to_owned();

    let mut strips: HashMap<String, [Vec<Frame>; 8]> = HashMap::new();
    let mut sounds = HashMap::new();
    let mut steps = HashSet::new();
    let mut apexes = HashSet::new();
    for (id, tile) in tileset.tiles() {
        if let Some(PropertyValue::StringValue(sfx)) = tile.properties.get("sfx") {
            sounds.insert(
                id,
                SfxId::by_name(sfx).unwrap_or_else(|error| panic!("actor sfx '{sfx}': {error}")),
            );
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
        sheet,
        frame: Size::new(tileset.tile_width as f32, tileset.tile_height as f32),
        columns: tileset.columns.max(1),
        hitbox: Size::new(dimension("hitbox_width"), dimension("hitbox_height")),
        airborne: matches!(
            tileset.properties.get("airborne"),
            Some(PropertyValue::BoolValue(true))
        ),
        strips,
        sounds,
        steps,
        apexes,
    }
}

pub struct ActorModel {
    sheet: String,
    frame: Size<WorldPx>,
    columns: u32,
    hitbox: Size<Tiles>,
    pub airborne: bool,
    strips: HashMap<String, [Vec<Frame>; 8]>,
    sounds: HashMap<TileId, SfxId>,
    steps: HashSet<TileId>,
    apexes: HashSet<TileId>,
}

impl ActorModel {
    pub fn hitbox(&self) -> Size<Tiles> {
        self.hitbox
    }

    pub fn sheet(&self) -> &str {
        &self.sheet
    }

    pub fn frame(
        &self,
        action: &str,
        dir: Direction,
        t: Seconds,
        attack_speed: PlaybackRate,
    ) -> Rect<WorldPx> {
        let strip = self.strip(action, dir);
        let elapsed = (t.millis() * rate(action, attack_speed)).max(Millis(0.0));
        let total = total_ms(strip);
        let position = if action == DEATH {
            elapsed.min(total - Millis(1.0))
        } else {
            elapsed % total
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

    pub fn timing(&self, action: &str, dir: Direction) -> Timing {
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
        dir: Direction,
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
        let authored = |t: Seconds| t.millis() * rate;
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

    fn strip(&self, action: &str, dir: Direction) -> &[Frame] {
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

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Timing {
    pub duration: Seconds,
    pub apex: Seconds,
}

const IDLE: &str = "idle";
const ATTACK: &str = "attack";
const DEATH: &str = "death";

fn dir_slot(dir: Direction) -> usize {
    dir as usize
}

fn rate(action: &str, attack_speed: PlaybackRate) -> PlaybackRate {
    if action == ATTACK {
        attack_speed.at_least(0.01)
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
            (t - at).ratio(total) as i64 + 1
        }
    };
    count(now) > count(prev)
}
