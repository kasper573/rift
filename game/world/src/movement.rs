//! Movement: an entity's replicated [`Position`] and the client's move requests. The server systems
//! that path-find and advance movers live in the `server` crate.

use bevy_app::App;
use bevy_ecs::entity::Entity;
use bevy_ecs::message::Message;
use bevy_ecs::prelude::Component;
use bevy_ecs::world::World;
use serde::{Deserialize, Serialize};

use crate::core::math::Pos;
use crate::core::tiling::Tiles;

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;

    app.replicate::<Position>()
        .add_client_message::<MoveRequest>(Channel::Ordered)
        .add_client_message::<MoveToPortal>(Channel::Ordered);
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Position {
    pub pos: Pos<Tiles>,
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MoveRequest {
    pub pos: Pos<Tiles>,
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MoveToPortal {
    pub pos: Pos<Tiles>,
    pub portal: u32,
}

pub fn position(world: &World, entity: Entity) -> Option<Pos<Tiles>> {
    world.get::<Position>(entity).map(|p| p.pos)
}
