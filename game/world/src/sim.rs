//! The host-only simulation layer: the systems that advance the authoritative world (NPC AI,
//! combat, movement, portals, rewards, visibility). It depends on the protocol, content, and asset
//! adapter beneath it and nothing depends back on it — the client never compiles this module (it is
//! gated behind the `host` feature), so the data contract it renders can never pull in server logic.

pub mod actions;
pub mod combat;
pub mod items;
pub mod movement;
pub mod npc;
pub mod player;
pub mod regen;
pub mod rewards;
pub mod spectate;
pub mod visibility;

use bevy_ecs::prelude::Bundle;
use bevy_replicon::prelude::Replicated;

use crate::protocol::{Actor, AreaTag, Hitbox, Name, Position, Vitals};
use combat::Stats;
use movement::Speed;

/// The components every character — player or NPC — shares. Ownership, inventory, AI and the like
/// are inserted alongside, per kind.
#[derive(Bundle)]
pub struct Character {
    pub replicated: Replicated,
    pub position: Position,
    pub name: Name,
    pub actor: Actor,
    pub hitbox: Hitbox,
    pub vitals: Vitals,
    pub area: AreaTag,
    pub stats: Stats,
    pub speed: Speed,
}

/// Forces every asset loader — actor models, areas, tables — so any broken file or dangling
/// reference panics. The server runs this at boot, refusing to start on bad content.
pub fn validate() {
    crate::actors::models();
    crate::area::areas();
    crate::items::items();
    npc::defs();
    npc::spawns();
    rewards::all();
    crate::sfx::sfx_table();
}

/// The authoritative simulation as an unfinished [`bevy_app::App`]: replication, the tick
/// schedule, and the connection observers are wired, but no transport is. The caller adds a
/// messaging backend (e.g. `RepliconRenetPlugins` + a `RenetServer`), inserts an `Identity` and
/// `ClientId` per connection, finishes the app, and calls `update()` at [`crate::TICK_HZ`].
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
    crate::protocol::protocol(&mut app);
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
