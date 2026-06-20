use bevy_app::App;
use bevy_ecs::entity::MapEntities;
use bevy_ecs::message::Message;
use bevy_ecs::prelude::*;
use bevy_replicon::prelude::*;
use serde::{Deserialize, Deserializer, Serialize};

use crate::actors::ActorModel;
use crate::area::AreaDef;
use crate::items::ItemDef;
use crate::math::{Pos, Size};
use crate::table::Id;
use crate::tiling::Tiles;
use crate::time::PlaybackRate;

pub fn protocol(app: &mut App) {
    app.replicate::<Position>()
        .replicate::<Actor>()
        .replicate::<Hitbox>()
        .replicate::<Vitals>()
        .replicate::<AreaTag>()
        .replicate::<Owner>()
        .replicate::<Name>()
        .replicate::<Spectate>()
        .replicate::<Xp>()
        .replicate::<Inventory>()
        .add_client_message::<JoinRequest>(Channel::Ordered)
        .add_client_message::<RespawnRequest>(Channel::Ordered)
        .add_client_message::<MoveRequest>(Channel::Ordered)
        .add_client_message::<MoveToPortal>(Channel::Ordered)
        .add_mapped_client_message::<AttackRequest>(Channel::Ordered)
        .add_client_message::<UseItemRequest>(Channel::Ordered)
        .add_client_message::<SpectateRequest>(Channel::Ordered)
        .add_server_message::<Welcome>(Channel::Ordered)
        .add_mapped_server_message::<ItemConsumed>(Channel::Ordered);
}

#[derive(
    Component,
    Serialize,
    Deserialize,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Debug,
    Default,
)]
#[component(immutable)]
pub struct ClientId(pub u32);

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
pub struct Position {
    pub pos: Pos<Tiles>,
}

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
pub struct Vitals {
    pub health: f32,
    pub max: f32,
}

impl Vitals {
    pub fn heal(&mut self, amount: f32) {
        self.health = (self.health + amount).min(self.max);
    }

    pub fn damage(&mut self, amount: f32) {
        self.health = (self.health - amount).max(0.0);
    }

    pub fn refill(&mut self) {
        self.health = self.max;
    }

    pub fn fraction(&self) -> f32 {
        (self.health / self.max).clamp(0.0, 1.0)
    }

    pub fn is_dead(&self) -> bool {
        self.health <= 0.0
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AreaTag {
    pub area: Id<AreaDef>,
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Owner {
    pub client: ClientId,
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Name {
    pub name: String,
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Spectate {
    pub watch: Option<ClientId>,
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Inventory {
    pub items: Vec<Id<ItemDef>>,
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Xp {
    pub amount: u32,
}

impl Xp {
    pub fn gain(&mut self, amount: u32) {
        self.amount += amount;
    }
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct JoinRequest;

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RespawnRequest;

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MoveRequest {
    pub pos: Pos<Tiles>,
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MoveToPortal {
    pub pos: Pos<Tiles>,
    pub portal: u32,
}

#[derive(Message, Serialize, Deserialize, MapEntities, Clone, Debug, PartialEq)]
pub struct AttackRequest {
    #[entities]
    pub target: Entity,
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct UseItemRequest {
    pub slot: u32,
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SpectateRequest {
    pub watch: Option<ClientId>,
}

#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Welcome {
    pub id: ClientId,
}

#[derive(Message, Serialize, Deserialize, MapEntities, Clone, Debug, PartialEq)]
pub struct ItemConsumed {
    pub item: Id<ItemDef>,
    #[entities]
    pub actor: Entity,
}

pub fn rgba_hex<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Rgba, D::Error> {
    let hex = String::deserialize(deserializer)?;
    hex.strip_prefix('#')
        .filter(|digits| digits.len() == 8)
        .and_then(|digits| u32::from_str_radix(digits, 16).ok())
        .map(Rgba)
        .ok_or_else(|| serde::de::Error::custom(format!("a color is #rrggbbaa, got '{hex}'")))
}

pub fn position(world: &World, entity: Entity) -> Option<Pos<Tiles>> {
    world.get::<Position>(entity).map(|p| p.pos)
}

pub fn is_dead(world: &World, entity: Entity) -> bool {
    world.get::<Vitals>(entity).is_some_and(Vitals::is_dead)
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
