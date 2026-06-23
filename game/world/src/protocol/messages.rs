//! The client⇄server messages: client requests (join, move, attack, …) and server announcements
//! (welcome, item consumed). Registered onto their channels in [`super::protocol`].

use bevy_ecs::entity::{Entity, MapEntities};
use bevy_ecs::message::Message;
use serde::{Deserialize, Serialize};

use super::components::ClientId;
use crate::content::items::ItemDef;
use crate::core::math::Pos;
use crate::core::table::Id;
use crate::core::tiling::Tiles;

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
