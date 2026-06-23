//! The authoritative server simulation: every gameplay feature's systems and server-only state,
//! composed into the per-area [`server_app`]. Built on the isomorphic `world` crate — server logic
//! lives here, not behind a feature flag in `world`.

pub mod actor;
pub mod combat;
pub mod items;
pub mod movement;
pub mod npc;
pub mod player;
pub mod rewards;
pub mod spectate;
pub mod transition;
pub mod visibility;

use bevy_app::App;
use bevy_ecs::prelude::{Bundle, Resource};
use bevy_replicon::prelude::Replicated;

use world::actor::{Actor, Hitbox, Name};
use world::area::{AreaDef, AreaTag};
use world::combat::Vitals;
use world::core::table::Id;
use world::movement::Position;

use crate::combat::Stats;
use crate::movement::Speed;

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
    world::actor::models();
    world::area::areas();
    world::items::items();
    npc::defs();
    npc::spawns();
    rewards::all();
    world::sfx::sfx_table();
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
    world::protocol(&mut app);
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
