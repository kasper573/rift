//! Actors: the replicated [`Actor`]/[`Hitbox`]/[`Name`] an entity shows in the world, the actions it
//! can play, and the [`ActorModel`] sprite-sheet catalog that drives its animation, frame timing, and
//! sound cues. Loading a model from a Tiled `.tsx` tileset lives in [`load`].

pub mod load;

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use bevy_app::App;
use bevy_ecs::prelude::*;
use serde::{Deserialize, Deserializer, Serialize};
use tiled::{Frame, TileId};

use crate::combat::Vitals;
use crate::core::assets;
use crate::core::math::{Pos, Rect, Size, WorldPx};
use crate::core::table::{Content, Id};
use crate::core::tiling::Tiles;
use crate::core::time::{Millis, PlaybackRate, Seconds};
use crate::sfx::SfxId;

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;

    app.replicate::<Actor>()
        .replicate::<Hitbox>()
        .replicate::<Name>();
}

pub const ACTION_IDLE: u8 = 0;
pub const ACTION_WALK: u8 = 1;
pub const ACTION_RUN: u8 = 2;
pub const ACTION_ATTACK: u8 = 3;
pub const ACTION_DEAD: u8 = 4;

pub fn action_name(action: u8) -> &'static str {
    match action {
        ACTION_WALK => "walk",
        ACTION_RUN => "run",
        ACTION_ATTACK => "attack",
        ACTION_DEAD => "death",
        _ => "idle",
    }
}

#[derive(
    Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default,
)]
pub struct Rgba(pub u32);

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Actor {
    pub color: Rgba,
    pub dir: u8,
    pub action: u8,
    pub model: Id<ActorModel>,
    pub attack_rate: PlaybackRate,
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Hitbox {
    pub size: Size<Tiles>,
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Name {
    pub name: String,
}

pub fn rgba_hex<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Rgba, D::Error> {
    let hex = String::deserialize(deserializer)?;
    hex.strip_prefix('#')
        .filter(|digits| digits.len() == 8)
        .and_then(|digits| u32::from_str_radix(digits, 16).ok())
        .map(Rgba)
        .ok_or_else(|| serde::de::Error::custom(format!("a color is #rrggbbaa, got '{hex}'")))
}

pub fn set_action(actor: &mut Mut<Actor>, action: u8) {
    if actor.action != action {
        actor.action = action;
    }
}

pub fn set_facing(actor: &mut Mut<Actor>, dir: u8, action: u8) {
    if actor.dir != dir || actor.action != action {
        actor.dir = dir;
        actor.action = action;
    }
}

/// Each tick, settle every actor back to idle (or dead) so a one-frame action set by a system this
/// tick wins; movement/combat re-assert walk/run/attack afterwards.
pub fn reset(mut actors: Query<(&mut Actor, Option<&Vitals>)>) {
    for (mut actor, vitals) in &mut actors {
        let dead = vitals.is_some_and(|v| v.health <= 0.0);
        set_action(&mut actor, if dead { ACTION_DEAD } else { ACTION_IDLE });
    }
}

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
