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

#[cfg(feature = "host")]
pub mod actions;
#[cfg(feature = "host")]
pub mod combat;
#[cfg(feature = "host")]
pub mod movement;
#[cfg(feature = "host")]
pub mod npc;
#[cfg(feature = "host")]
pub mod player;
#[cfg(feature = "host")]
pub mod regen;
#[cfg(feature = "host")]
pub mod rewards;
#[cfg(feature = "host")]
pub mod spectate;
#[cfg(feature = "host")]
pub mod visibility;

pub use bevy_ecs::entity::Entity;
pub use bevy_ecs::query::With;
pub use bevy_ecs::world::World;

pub use crate::identity::Identity;
pub use crate::protocol::{
    ACTION_ATTACK, ACTION_DEAD, ACTION_IDLE, ACTION_RUN, ACTION_WALK, Actor, AreaTag,
    AttackRequest, ClientId, Hitbox, Inventory, ItemConsumed, ItemId, JoinRequest, MoveRequest,
    MoveToPortal, Name, Owner, Position, RespawnRequest, Rgba, Spectate, SpectateRequest,
    UseItemRequest, Vitals, Welcome, Xp,
};
pub use crate::role::Role;

pub const TICK_HZ: math::Hertz = math::Hertz(30.0);

/// Forces every asset loader — actor models, areas, tables — so any broken file or dangling
/// reference panics. The server runs this at boot, refusing to start on bad content.
#[cfg(feature = "host")]
pub fn validate() {
    actors::models();
    area::areas();
    items::items();
    npc::defs();
    npc::spawns();
    rewards::all();
    sfx::sfx_table();
}

/// The authoritative simulation as an unfinished [`bevy_app::App`]: replication, the tick
/// schedule, and the connection observers are wired, but no transport is. The caller adds a
/// messaging backend (e.g. `RepliconRenetPlugins` + a `RenetServer`), inserts an [`Identity`] and
/// [`ClientId`] per connection, finishes the app, and calls `update()` at [`TICK_HZ`].
#[cfg(feature = "host")]
pub fn server_app() -> bevy_app::App {
    use bevy_app::{Startup, Update};
    use bevy_ecs::schedule::IntoScheduleConfigs;
    use bevy_replicon::prelude::{AuthMethod, RepliconSharedPlugin};

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
    protocol::protocol(&mut app);
    visibility::register(&mut app);
    app.init_resource::<player::Players>()
        .init_resource::<spectate::Spectators>()
        .init_resource::<regen::RegenAt>()
        .add_message::<combat::Died>()
        .add_observer(player::greet)
        .add_observer(player::client_left)
        .add_observer(spectate::client_left)
        .add_systems(Startup, npc::spawn_all)
        .add_systems(
            Update,
            (
                actions::reset,
                regen::regen,
                npc::run_ai,
                movement::move_request,
                movement::move_to_portal,
                combat::request,
                combat::combat,
                items::use_item,
                rewards::grant,
                movement::advance,
                player::join,
                player::respawn,
                spectate::requests,
                spectate::follow,
                npc::run_respawn,
                visibility::update,
            )
                .chain(),
        );
    app
}
