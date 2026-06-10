pub mod core;
pub mod features;

pub use bevy_ecs::entity::Entity;
pub use bevy_ecs::query::With;
pub use bevy_ecs::world::World;
#[cfg(feature = "host")]
pub use bevy_replicon::prelude::{ConnectedClient, ServerMessages};

pub use crate::core::identity::Identity;
pub use crate::core::protocol::{
    ACTION_ATTACK, ACTION_DEAD, ACTION_IDLE, ACTION_RUN, ACTION_WALK, Actor, AreaTag,
    AttackRequest, ClientId, Hitbox, Inventory, ItemConsumed, ItemId, JoinRequest, MoveRequest,
    MoveToPortal, Name, Owner, Position, RespawnRequest, Rgba, SPECTATE_ROLE, Spectate,
    SpectateRequest, UseItemRequest, Vitals, Welcome, Xp,
};
pub use crate::core::session::{LinkStatus, MmoClient, Transport};

pub const TICK_HZ: f32 = 30.0;

pub const DEFAULT_ADDRESS: &str = "127.0.0.1:9998";

/// Forces every asset loader — actor models, areas, tables — so any broken file or dangling
/// reference panics. The server runs this at boot, refusing to start on bad content.
#[cfg(feature = "host")]
pub fn validate() {
    core::actors::models();
    core::area::areas();
    features::items::items();
    features::npc::defs();
    features::npc::spawns();
    features::rewards::all();
    features::sfx::sfx_table();
}

/// The fully assembled authoritative simulation, already running; the caller owns the transport:
/// spawn a [`bevy_replicon::prelude::ConnectedClient`] (with [`ClientId`] and [`Identity`]) per
/// connection, shuttle byte frames through [`bevy_replicon::prelude::ServerMessages`], and call
/// `update()` at [`TICK_HZ`].
#[cfg(feature = "host")]
pub fn server_app() -> bevy_app::App {
    use bevy_replicon::prelude::{AuthMethod, RepliconSharedPlugin, ServerState};

    let mut app = bevy_app::App::new();
    app.add_plugins((bevy_time::TimePlugin, bevy_state::app::StatesPlugin));
    app.add_plugins(
        bevy_app::PluginGroup::build(bevy_replicon::prelude::RepliconPlugins)
            .set(RepliconSharedPlugin {
                auth_method: AuthMethod::None,
            })
            .set(bevy_replicon::server::ServerPlugin::new(
                bevy_app::PostUpdate,
            )),
    );
    core::protocol::protocol(&mut app);
    features::features(&mut app);
    app.finish();
    app.world_mut()
        .resource_mut::<bevy_state::prelude::NextState<ServerState>>()
        .set(ServerState::Running);
    app
}
