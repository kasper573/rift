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

use actor::{Actor, Hitbox, Name};
use area::AreaTag;
use movement::Position;

pub const TICK_HZ: crate::core::time::Hertz = crate::core::time::Hertz(30.0);

pub(crate) fn requests<M: Message>(world: &mut World) -> Vec<FromClient<M>> {
    world
        .resource_mut::<Messages<FromClient<M>>>()
        .drain()
        .collect()
}

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

#[derive(Resource, Clone, Copy)]
pub struct WorldArea(pub area::Id);

#[derive(Bundle)]
pub struct Character {
    pub replicated: Replicated,
    pub position: Position,
    pub name: Name,
    pub actor: Actor,
    pub hitbox: Hitbox,
    pub area: AreaTag,
}

pub fn server_app(area: area::Id) -> App {
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
