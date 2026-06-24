use bevy::asset::AssetApp;
use bevy::asset::io::AssetSourceId;
use bevy::prelude::*;
use world::systems::account::Role;

pub mod core;
pub mod systems;

#[derive(States, Default, Debug, Clone, PartialEq, Eq, Hash)]
pub enum GameScene {
    /// Spectators choose whether to play or watch; players skip straight to [`GameScene::Playing`].
    #[default]
    ChooseMode,
    Playing,
}

/// Builds and runs the Bevy app: reads the boot params from the platform, builds the session from the
/// access token, and starts the app. The platform's entry point calls this.
pub fn boot() {
    let params = core::platform::read_start_params();
    let session = params
        .access_token
        .as_deref()
        .map(core::net::auth::Session::from_access_token);
    let spectator = session
        .as_ref()
        .is_some_and(|session| session.roles.contains(&Role::Spectate));

    let mut app = App::new();
    app.register_asset_source(AssetSourceId::Default, core::assets::embedded_source())
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
        .insert_state(if spectator {
            GameScene::ChooseMode
        } else {
            GameScene::Playing
        })
        .add_plugins((
            ui::UiPlugin,
            core::net::NetPlugin,
            core::render::RenderPlugin,
            core::audio::SfxPlugin,
            core::debug::DebugPlugin,
            core::testing::TestingPlugin,
            systems::actor::ActorPlugin,
            systems::area::AreaPlugin,
            systems::overlay::OverlayPlugin,
            systems::input::InputPlugin,
            systems::hud::scenes::ScenesPlugin,
            systems::hud::connection::ConnectionPlugin,
            systems::hud::HudPlugin,
            systems::hud::fps::FpsPlugin,
        ));
    ui::theme::set_theme(ui::themes::dark::THEME);
    app.run();
}
