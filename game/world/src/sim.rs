//! Host-only simulation layer; gated behind the `host` feature so the protocol cannot depend on server logic.

pub mod actions;
pub mod combat;
pub mod items;
pub mod movement;
pub mod npc;
pub mod player;
pub mod regen;
pub mod rewards;
pub mod spectate;
pub mod transition;
pub mod visibility;

use bevy_ecs::prelude::{Bundle, Resource};
use bevy_replicon::prelude::Replicated;

use crate::area::AreaDef;
use crate::protocol::{Actor, AreaTag, Hitbox, Name, Position, Vitals};
use crate::table::Id;
use combat::Stats;
use movement::Speed;

/// Each world hosts exactly one area; crossing a portal hands the player off to the world hosting the destination area.
#[derive(Resource, Clone, Copy)]
pub struct HostedArea(pub Id<AreaDef>);

/// When inserted with `false`, an area starts empty of NPCs. The e2e disables them so its idle player
/// can cross the island to a portal without being attacked; absent, areas populate as normal.
#[derive(Resource, Clone, Copy)]
pub struct SpawnNpcs(pub bool);

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

pub fn validate() {
    crate::actors::models();
    crate::area::areas();
    crate::items::items();
    npc::defs();
    npc::spawns();
    rewards::all();
    crate::sfx::sfx_table();
}

pub fn server_app(area: Id<AreaDef>) -> bevy_app::App {
    use bevy_app::{Startup, Update};
    use bevy_ecs::schedule::IntoScheduleConfigs;
    use bevy_replicon::prelude::{AuthMethod, RepliconSharedPlugin};

    let mut app = bevy_app::App::new();
    app.insert_resource(HostedArea(area));
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
