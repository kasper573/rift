pub mod core;
pub mod features;

pub use rift::{App, ClientId, Entity, LinkStatus, Transport};

pub use crate::core::identity::Identity;
pub use crate::core::protocol::{
    ACTION_ATTACK, ACTION_DEAD, ACTION_IDLE, ACTION_RUN, ACTION_WALK, Actor, AreaTag, Hitbox,
    Inventory, ItemId, Name, Owner, Position, Rgba, Spectate, UseItemRequest, Vitals, Xp,
};
pub use crate::core::session::MmoClient;
pub use crate::features::combat::AttackRequest;
pub use crate::features::items::ItemConsumed;
pub use crate::features::movement::{MoveRequest, MoveToPortal};
pub use crate::features::player::{JoinRequest, RespawnRequest};
pub use crate::features::spectate::{SPECTATE_ROLE, SpectateRequest};
pub use crate::features::visibility::VIEW_DISTANCE;

pub const TICK_HZ: f32 = 30.0;

pub const DEFAULT_ADDRESS: &str = "127.0.0.1:9998";

// rift's sharding API speaks raw zone numbers, so the AreaId boundary unwraps here.
pub fn spawn_zone() -> u32 {
    crate::core::area::spawn_zone().0
}

/// Forces every asset loader — actor models, areas, tables — so any broken file or dangling
/// reference panics. The server's build script runs this, failing the build on bad content.
pub fn validate() {
    core::actors::models();
    core::area::areas();
    features::items::items();
    features::npc::defs();
    features::npc::spawns();
    features::rewards::all();
    features::sfx::sfx_table();
}

pub fn features() -> Vec<rift::Feature> {
    crate::features::all()
}

// One shard per zone: each area is its own world that ticks independently.
pub fn zones() -> Vec<u32> {
    crate::core::area::areas()
        .iter()
        .map(|area| area.id.0)
        .collect()
}
