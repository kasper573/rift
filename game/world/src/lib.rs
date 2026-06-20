pub mod actors;
pub mod area;
pub mod assets;
pub mod identity;
pub mod items;
pub mod math;
pub mod nav;
pub mod protocol;
pub mod role;
pub mod session;
pub mod sfx;
pub mod table;
pub mod tiling;
pub mod time;

#[cfg(feature = "host")]
pub mod sim;

pub use bevy_ecs::entity::Entity;
pub use bevy_ecs::query::With;
pub use bevy_ecs::world::World;

pub use crate::identity::Identity;
pub use crate::protocol::{
    ACTION_ATTACK, ACTION_DEAD, ACTION_IDLE, ACTION_RUN, ACTION_WALK, Actor, AreaTag,
    AttackRequest, ClientId, Hitbox, Inventory, ItemConsumed, JoinRequest, MoveRequest,
    MoveToPortal, Name, Owner, Position, RespawnRequest, Rgba, Spectate, SpectateRequest,
    UseItemRequest, Vitals, Welcome, Xp,
};
pub use crate::role::Role;

pub const TICK_HZ: time::Hertz = time::Hertz(30.0);
