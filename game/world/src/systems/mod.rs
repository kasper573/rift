//! The game itself: every gameplay feature ([`actor`], [`movement`], [`combat`], [`npc`], [`player`],
//! [`items`], [`rewards`], [`spectate`], [`area`], [`visibility`], plus [`account`] identity and the
//! [`sfx`] catalog), each owning its replicated components, client⇄server messages, content, and the
//! systems that drive it. This module also composes them into the headless [`server_app`]. Built on
//! the game-agnostic [`crate::core`] substrate.

pub mod account;
pub mod actor;
pub mod area;
pub mod combat;
pub mod items;
pub mod movement;
pub mod npc;
pub mod player;
pub mod rewards;
pub mod sfx;
pub mod spectate;
pub mod visibility;

use bevy_app::App;
use bevy_ecs::prelude::{Bundle, Resource};
use bevy_replicon::prelude::Replicated;

use crate::core::table::Id;
use actor::{Actor, Hitbox, Name};
use area::{AreaDef, AreaTag};
use combat::{Stats, Vitals};
use movement::{Position, Speed};

pub const TICK_HZ: crate::core::time::Hertz = crate::core::time::Hertz(30.0);

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

/// Each world runs exactly one area; crossing a portal hands the player off to the world running the
/// destination area.
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
    sfx::sfx_table();
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
    protocol(&mut app);
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
                items::drop_item,
                items::pickup_request,
                rewards::grant,
                movement::advance,
                items::pickups,
                items::expire_drops,
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
