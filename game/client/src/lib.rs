use bevy::asset::AssetApp;
use bevy::asset::io::AssetSourceId;
use bevy::prelude::*;
use world::core::math::Rng;
use world::systems::account::Role;

pub mod core;
pub mod systems;

pub use systems::scene::Scene;

pub fn boot() {
    let params = core::platform::read_start_params();
    let session = params
        .access_token
        .as_deref()
        .map(core::net::auth::Session::from_access_token);
    let spectator = session.as_ref().is_some_and(|session| {
        session
            .roles
            .iter()
            .any(|role| role.parse() == Ok(Role::Spectate))
    });

    let mut app = App::new();
    app.insert_resource(core::assets::service());
    app.insert_resource(Rng::from_entropy());
    app.register_asset_source(AssetSourceId::Default, core::assets::bevy_source())
        .add_plugins(
            DefaultPlugins
                .set(bevy::log::LogPlugin {
                    filter: format!("{},symphonia=warn", bevy::log::DEFAULT_FILTER),
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: Some(core::platform::primary_window()),
                    ..default()
                })
                .set(ImagePlugin::default_nearest()),
        );
    if let Some(session) = session {
        app.insert_resource(session);
    }
    app.insert_resource(params)
        .add_plugins((
            ui::UiPlugin,
            core::net::NetPlugin,
            core::render::RenderPlugin,
            core::sfx::SfxPlugin,
        ))
        .add_plugins((
            systems::scene::ScenePlugin { spectator },
            systems::actor::ActorPlugin,
            systems::interpolate::InterpolatePlugin,
            systems::combat::CombatPlugin,
            systems::item::ItemsPlugin,
            systems::view::ViewPlugin,
            systems::session::SessionPlugin,
            systems::input::InputPlugin,
            systems::debug::DebugPlugin,
            systems::testing::TestingPlugin,
            systems::widget::HudPlugin,
            systems::fps::FpsPlugin,
        ));
    ui::theme::set_theme(ui::themes::dark::THEME);
    app.run();
}
