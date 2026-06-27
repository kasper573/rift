mod model;

use bevy_app::App;
use bevy_ecs::prelude::*;
use serde::{Deserialize, Deserializer, Serialize};

use crate::core::math::{Direction, Size};
use crate::core::tiling::Tiles;
use crate::core::time::PlaybackRate;
use crate::data;
use crate::systems::stat::{StatKind, Stats};

pub use model::{ActorModel, Timing, load};

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;

    app.replicate::<Actor>()
        .replicate::<Hitbox>()
        .replicate::<Name>();
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Action {
    #[default]
    Idle,
    Walk,
    Run,
    Attack,
    Dead,
}

impl Action {
    pub fn name(self) -> &'static str {
        match self {
            Action::Idle => "idle",
            Action::Walk => "walk",
            Action::Run => "run",
            Action::Attack => "attack",
            Action::Dead => "death",
        }
    }
}

#[derive(
    Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default,
)]
pub struct Rgba(pub u32);

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Actor {
    pub color: Rgba,
    pub dir: Direction,
    pub action: Action,
    pub model: data::model::Id,
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

pub fn set_action(actor: &mut Mut<Actor>, action: Action) {
    if actor.action != action {
        actor.action = action;
    }
}

pub fn set_facing(actor: &mut Mut<Actor>, dir: Direction, action: Action) {
    if actor.dir != dir || actor.action != action {
        actor.dir = dir;
        actor.action = action;
    }
}

pub fn reset(mut actors: Query<(&mut Actor, Option<&Stats>)>) {
    for (mut actor, stats) in &mut actors {
        let dead = stats.is_some_and(|stats| stats.get(StatKind::Health) <= 0.0);
        set_action(&mut actor, if dead { Action::Dead } else { Action::Idle });
    }
}
