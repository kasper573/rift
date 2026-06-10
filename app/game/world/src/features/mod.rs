pub mod items;
pub mod sfx;

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

#[cfg(feature = "host")]
pub fn features(app: &mut bevy_app::App) {
    use bevy_app::{Startup, Update};
    use bevy_ecs::schedule::IntoScheduleConfigs;

    visibility::register(app);
    app.init_resource::<player::Players>()
        .init_resource::<spectate::Spectators>()
        .init_resource::<regen::RegenAt>()
        .add_message::<combat::Died>()
        .add_observer(player::greet)
        .add_observer(player::client_left)
        .add_observer(spectate::client_left)
        .add_systems(Startup, npc::spawn_all)
        // The chain is run order: reset → regen → npc ai → intents → combat → items → rewards →
        // movement → join/respawn → spectate → npc respawn → visibility.
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
}
