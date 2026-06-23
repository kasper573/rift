pub mod account;
pub mod content;
pub mod core;
pub mod protocol;

#[cfg(feature = "systems")]
pub mod systems;

pub use bevy_ecs::entity::Entity;
pub use bevy_ecs::query::With;
pub use bevy_ecs::world::World;

pub use crate::account::{Identity, Role};
pub use crate::protocol::{
    ACTION_ATTACK, ACTION_DEAD, ACTION_IDLE, ACTION_RUN, ACTION_WALK, Actor, AreaTag,
    AttackRequest, ClientId, Hitbox, Inventory, ItemConsumed, JoinRequest, MoveRequest,
    MoveToPortal, Name, Owner, Position, RespawnRequest, Rgba, Spectate, SpectateRequest,
    UseItemRequest, Vitals, Welcome, Xp,
};

pub const TICK_HZ: core::time::Hertz = core::time::Hertz(30.0);
