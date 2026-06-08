use rift::{ClientId, Entity, Wire, World};
use serde::{Deserialize, Deserializer};

use crate::core::actors::ActorModelId;
use crate::core::area::AreaId;
use crate::core::math::{PlaybackRate, Pos, Size, Tiles};

pub const ACTION_IDLE: u8 = 0;
pub const ACTION_WALK: u8 = 1;
pub const ACTION_RUN: u8 = 2;
pub const ACTION_ATTACK: u8 = 3;
pub const ACTION_DEAD: u8 = 4;

/// The wire action's verb in the actor model manifests; unknown actions read as idle.
pub fn action_name(action: u8) -> &'static str {
    match action {
        ACTION_WALK => "walk",
        ACTION_RUN => "run",
        ACTION_ATTACK => "attack",
        ACTION_DEAD => "death",
        _ => "idle",
    }
}

/// A `0xRRGGBBAA` packed color; content tables spell it `#rrggbbaa`.
#[derive(Wire, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct Rgba(pub u32);

/// An item definition's index in [`crate::features::items::items`].
#[derive(Wire, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct ItemId(pub u16);

#[derive(Wire, Clone, Debug, PartialEq)]
pub struct Position {
    pub pos: Pos<Tiles>,
}

#[derive(Wire, Clone, Debug, PartialEq)]
pub struct Actor {
    pub color: Rgba,
    pub dir: u8,
    pub action: u8,
    pub model: ActorModelId,
    pub attack_rate: PlaybackRate,
}

/// Click target in tiles: a box centered on x with its bottom at the feet line.
#[derive(Wire, Clone, Debug, PartialEq)]
pub struct Hitbox {
    pub size: Size<Tiles>,
}

#[derive(Wire, Clone, Debug, PartialEq)]
pub struct Vitals {
    pub health: f32,
    pub max: f32,
}

#[derive(Wire, Clone, Debug, PartialEq)]
pub struct AreaTag {
    pub area: AreaId,
}

#[derive(Wire, Clone, Debug, PartialEq)]
pub struct Owner {
    pub client: ClientId,
}

#[derive(Wire, Clone, Debug, PartialEq)]
pub struct Name {
    pub name: String,
}

/// A spectator's camera anchor; `watch: None` is free spectating.
#[derive(Wire, Clone, Debug, PartialEq)]
pub struct Spectate {
    pub watch: Option<ClientId>,
}

/// Each element is one owned item instance.
#[derive(Wire, Clone, Debug, PartialEq)]
pub struct Inventory {
    pub items: Vec<ItemId>,
}

#[derive(Wire, Clone, Debug, PartialEq)]
pub struct Xp {
    pub amount: u32,
}

#[derive(Wire, Clone, Debug, PartialEq)]
pub struct UseItemRequest {
    pub slot: u32,
}

impl<'de> Deserialize<'de> for Rgba {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let hex = String::deserialize(deserializer)?;
        hex.strip_prefix('#')
            .filter(|digits| digits.len() == 8)
            .and_then(|digits| u32::from_str_radix(digits, 16).ok())
            .map(Rgba)
            .ok_or_else(|| serde::de::Error::custom(format!("a color is #rrggbbaa, got '{hex}'")))
    }
}

pub fn position(world: &World, entity: Entity) -> Option<Pos<Tiles>> {
    world.get::<Position>(entity).map(|p| p.pos)
}

pub fn set_action(world: &mut World, entity: Entity, action: u8) {
    if world.get::<Actor>(entity).map(|a| a.action) != Some(action) {
        world.modify::<Actor>(entity, |a| a.action = action);
    }
}

pub fn set_facing(world: &mut World, entity: Entity, dir: u8, action: u8) {
    if world.get::<Actor>(entity).map(|a| (a.dir, a.action)) != Some((dir, action)) {
        world.modify::<Actor>(entity, |a| {
            a.dir = dir;
            a.action = action;
        });
    }
}

pub fn is_dead(world: &World, entity: Entity) -> bool {
    world.get::<Vitals>(entity).is_some_and(|v| v.health <= 0.0)
}
