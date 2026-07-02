use bevy::asset::AssetApp;
use bevy::asset::io::AssetSourceId;
use bevy::prelude::*;
use game::core::math::Rng;
use game::systems::account::Role;
use game::{core, systems};
use wasm_bindgen::prelude::wasm_bindgen;

#[wasm_bindgen(module = "/src/bin/client/audio-unlock.js")]
unsafe extern "C" {
    fn audio_unlock();
}

pub fn run() {
    console_error_panic_hook::set_once();
    audio_unlock();
    boot();
}

fn boot() {
    let params = crate::platform::read_start_params();
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
    app.insert_resource(crate::assets::service());
    app.insert_resource(Rng::from_entropy());
    app.register_asset_source(AssetSourceId::Default, crate::assets::bevy_source())
        .add_plugins(
            DefaultPlugins
                .set(bevy::log::LogPlugin {
                    filter: format!("{},symphonia=warn", bevy::log::DEFAULT_FILTER),
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: Some(crate::platform::primary_window()),
                    ..default()
                })
                .set(ImagePlugin::default_nearest()),
        );
    if let Some(session) = session {
        app.insert_resource(session);
    }
    app.insert_resource(params)
        .insert_resource(core::platform::ClientPlatform(Box::new(
            crate::platform::WebPlatform,
        )))
        .add_plugins((
            ui::UiPlugin,
            core::net::transport::RepliconRenetClientPlugin,
            core::render::RenderPlugin,
            core::sfx::playback::SfxPlugin,
        ))
        .add_plugins((
            systems::scene::ScenePlugin { spectator },
            systems::actor::render::ActorPlugin,
            systems::movement::MovementPlugin,
            systems::combat::render::CombatPlugin,
            systems::item::render::ItemsPlugin,
            systems::view::ViewPlugin,
            systems::player::session::ClientSessionPlugin,
            systems::input::InputPlugin,
            systems::debug::DebugPlugin,
            crate::testing::TestingPlugin,
            systems::hud::HudPlugin,
            systems::terminal::TerminalPlugin,
            systems::fps::FpsPlugin,
        ));
    ui::theme::set_theme(ui::themes::dark::THEME);
    app.run();
}
