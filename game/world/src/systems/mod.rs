//! The game itself: every gameplay feature owns its replicated components, client⇄server messages,
//! content, and driving systems, composed into the headless [`server_app`]. Built on the
//! game-agnostic [`crate::core`] substrate.

pub mod account;
pub mod actor;
pub mod area;
pub mod combat;
pub mod effect;
pub mod equipment;
pub mod item;
pub mod job;
pub mod movement;
pub mod npc;
pub mod player;
pub mod rewards;
pub mod sfx;
pub mod spectate;
pub mod stat;
pub mod visibility;

use bevy_app::App;
use bevy_ecs::message::{Message, Messages};
use bevy_ecs::prelude::{Bundle, Resource, World};
use bevy_replicon::prelude::{FromClient, Replicated};

use crate::core::table::Id;
use actor::{Actor, Hitbox, Name};
use area::{AreaDef, AreaTag};
use movement::Position;

pub const TICK_HZ: crate::core::time::Hertz = crate::core::time::Hertz(30.0);

/// Drains this tick's buffered client requests of message type `M`. Every request-handling system
/// funnels through here, so the drain-and-collect incantation has one home.
pub(crate) fn requests<M: Message>(world: &mut World) -> Vec<FromClient<M>> {
    world
        .resource_mut::<Messages<FromClient<M>>>()
        .drain()
        .collect()
}

/// Registers every feature's replicated components and client⇄server messages. Both the client
/// session and the server app call this so the two sides agree on the wire.
pub fn protocol(app: &mut App) {
    actor::register(app);
    area::register(app);
    combat::register(app);
    stat::register(app);
    effect::register(app);
    equipment::register(app);
    item::register(app);
    job::register(app);
    movement::register(app);
    npc::register(app);
    player::register(app);
    spectate::register(app);
}

/// Each world runs exactly one area; crossing a portal hands the player off to the world running the
/// destination area.
#[derive(Resource, Clone, Copy)]
pub struct WorldArea(pub Id<AreaDef>);

/// The common, non-stat components every actor replicates. Stats are separate per-stat components
/// authored from a [`stat::StatSet`] right after spawn (see `npc::spawn`/`player::place`).
#[derive(Bundle)]
pub struct Character {
    pub replicated: Replicated,
    pub position: Position,
    pub name: Name,
    pub actor: Actor,
    pub hitbox: Hitbox,
    pub area: AreaTag,
}

/// Forces every content table to load and validate, independent of any running app.
pub fn validate() {
    actor::models();
    area::areas();
    item::items();
    job::defs();
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
            // Split into two chained groups: a tick has more systems than `chain` takes in one tuple.
            // Effective stats are read on demand, so combat/movement need no recompute pass.
            (
                (
                    actor::reset,
                    combat::regen,
                    npc::run_ai,
                    movement::move_request,
                    movement::move_to_portal,
                    combat::request,
                    item::use_item,
                    item::drop_item,
                    item::pickup_request,
                    equipment::unequip,
                    effect::expire,
                    combat::combat,
                )
                    .chain(),
                (
                    rewards::grant,
                    movement::advance,
                    item::pickups,
                    item::expire_drops,
                    player::join,
                    player::respawn,
                    spectate::requests,
                    spectate::follow,
                    npc::run_respawn,
                    visibility::update,
                )
                    .chain(),
            )
                .chain(),
        );
    app
}
