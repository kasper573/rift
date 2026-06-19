// The data contract the client and server share: the wire protocol, the content tables, and the
// injected asset adapter that loads them. Compiles on its own — the client builds exactly this.
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

// The host-only simulation layered on top of the contract above; gated out of the client entirely.
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

pub const TICK_HZ: math::Hertz = math::Hertz(30.0);
