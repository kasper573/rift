//! The wire contract: every replicated component and client/server message, registered
//! identically on both sides so bevy_replicon's channels line up. The server replicates
//! component changes each tick; clients send intents as messages, and entity-bearing payloads
//! are remapped between the two worlds automatically.

use bevy_app::App;
use bevy_ecs::entity::MapEntities;
use bevy_ecs::message::Message;
use bevy_ecs::prelude::*;
use bevy_replicon::prelude::*;
use serde::{Deserialize, Deserializer, Serialize};

use crate::actors::ActorModelId;
use crate::area::AreaId;
use crate::math::{PlaybackRate, Pos, Size, Tiles};

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

/// One connection's stable id, assigned by the transport; on the server it sits on the
/// connection's client entity.
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

/// The JWT role that entitles a client to spectate.
pub const SPECTATE_ROLE: &str = "spectate";

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

/// A `0xRRGGBBAA` packed color; content tables spell it `#rrggbbaa` via [`rgba_hex`].
#[derive(
    Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default,
)]
pub struct Rgba(pub u32);

/// An item definition's index in [`crate::items::items`].
#[derive(
    Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default,
)]
pub struct ItemId(pub u16);

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Position {
    pub pos: Pos<Tiles>,
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Actor {
    pub color: Rgba,
    pub dir: u8,
    pub action: u8,
    pub model: ActorModelId,
    pub attack_rate: PlaybackRate,
}

/// Click target in tiles: a box centered on x with its bottom at the feet line.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Hitbox {
    pub size: Size<Tiles>,
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Vitals {
    pub health: f32,
    pub max: f32,
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AreaTag {
    pub area: AreaId,
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Owner {
    pub client: ClientId,
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Name {
    pub name: String,
}

/// A spectator's camera anchor; `watch: None` is free spectating.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Spectate {
    pub watch: Option<ClientId>,
}

/// Each element is one owned item instance; replicated only to the owning client.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Inventory {
    pub items: Vec<ItemId>,
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Xp {
    pub amount: u32,
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

/// The server's hello: the connection's [`ClientId`], which [`Owner`] components refer to.
#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Welcome {
    pub id: ClientId,
}

/// Announces a consumed inventory item so clients can play its sound at the consumer.
#[derive(Message, Serialize, Deserialize, MapEntities, Clone, Debug, PartialEq)]
pub struct ItemConsumed {
    pub item: ItemId,
    #[entities]
    pub actor: Entity,
}

/// Deserializes an [`Rgba`] from a content table's `#rrggbbaa` spelling.
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
    world.get::<Vitals>(entity).is_some_and(|v| v.health <= 0.0)
}

/// Writes only on change, so replication ships [`Actor`] exactly when it really moved.
pub fn set_action(actor: &mut Mut<Actor>, action: u8) {
    if actor.action != action {
        actor.action = action;
    }
}

/// Writes only on change, so replication ships [`Actor`] exactly when it really moved.
pub fn set_facing(actor: &mut Mut<Actor>, dir: u8, action: u8) {
    if actor.dir != dir || actor.action != action {
        actor.dir = dir;
        actor.action = action;
    }
}
