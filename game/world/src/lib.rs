//! The MMORPG game, headless. This crate *is* the game — its replicated components, the client⇄server
//! messages and their registration, the static content, and the simulation systems that advance the
//! world — organized by feature ([`actor`], [`movement`], [`combat`], [`npc`], [`player`], [`items`],
//! [`rewards`], [`spectate`], [`area`], [`visibility`]) over the game-agnostic [`core`] substrate.
//! The `client` crate wraps a frontend (rendering/input) around it and the `server` crate a backend
//! (netcode/HTTP); a benchmark can consume it standalone to measure the headless tick.

pub mod account;
pub mod actor;
pub mod area;
pub mod channels;
pub mod combat;
pub mod core;
pub mod items;
pub mod movement;
pub mod npc;
pub mod player;
pub mod rewards;
pub mod sfx;
pub mod spectate;
pub mod visibility;

use bevy_app::App;

pub use app::{Character, WorldArea, server_app, validate};

pub const TICK_HZ: core::time::Hertz = core::time::Hertz(30.0);

/// Registers every feature's replicated components and client⇄server messages. Both the client
/// session and the server app call this so the two sides agree on the wire.
pub fn protocol(app: &mut App) {
    actor::register(app);
    area::register(app);
    combat::register(app);
    items::register(app);
    movement::register(app);
    player::register(app);
    spectate::register(app);
}

/// Assembles the whole game into one headless [`App`]: every feature's systems in tick order, the
/// replication registration, and the resources/observers they need. The server runs one per area; a
/// benchmark drives it directly.
mod app {
    use bevy_app::App;
    use bevy_ecs::prelude::{Bundle, Resource};
    use bevy_replicon::prelude::Replicated;

    use crate::actor::{Actor, Hitbox, Name};
    use crate::area::{AreaDef, AreaTag};
    use crate::combat::{Stats, Vitals};
    use crate::core::table::Id;
    use crate::movement::{Position, Speed};
    use crate::{actor, area, combat, items, movement, npc, player, rewards, spectate, visibility};

    /// Each world runs exactly one area; crossing a portal hands the player off to the world running
    /// the destination area.
    #[derive(Resource, Clone, Copy)]
    pub struct WorldArea(pub Id<AreaDef>);

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

    /// Forces every content table to load and validate, independent of any running app.
    pub fn validate() {
        actor::models();
        area::areas();
        items::items();
        npc::defs();
        npc::spawns();
        rewards::all();
        crate::sfx::sfx_table();
    }

    pub fn server_app(area: Id<AreaDef>) -> App {
        use bevy_app::{Startup, Update};
        use bevy_ecs::schedule::IntoScheduleConfigs;
        use bevy_replicon::prelude::{AuthMethod, RepliconSharedPlugin};

        let mut app = App::new();
        app.insert_resource(WorldArea(area));
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
        crate::protocol(&mut app);
        visibility::register(&mut app);
        app.init_resource::<player::Players>()
            .init_resource::<spectate::Spectators>()
            .init_resource::<combat::RegenAt>()
            .add_message::<combat::Died>()
            .add_observer(player::greet)
            .add_observer(player::client_left)
            .add_observer(spectate::client_left)
            .add_systems(Startup, npc::spawn_all)
            .add_systems(
                Update,
                (
                    actor::reset,
                    combat::regen,
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
}
