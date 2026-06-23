//! Actor models: a sprite sheet's per-direction animation strips, hitbox, and sound cues, with the
//! runtime that samples a frame, attack timing, and step/apex cues. Loading from a Tiled `.tsx`
//! tileset lives in [`load`].

pub mod load;

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use serde::Deserialize;
use tiled::{Frame, TileId};

use crate::core::assets;
use crate::core::math::{Pos, Rect, Size, WorldPx};
use crate::core::table::Content;
use crate::core::tiling::Tiles;
use crate::core::time::{Millis, PlaybackRate, Seconds};

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
            .map(|path| load::load(assets::stem(path)))
            .collect();
        all.sort_unstable_by(|a, b| a.name.cmp(&b.name));
        all
    })
}

const IDLE: &str = "idle";
const ATTACK: &str = "attack";
const DEATH: &str = "death";

fn dir_slot(dir: u8) -> usize {
    if dir > 7 { 0 } else { dir as usize }
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
