use base64::Engine;
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
    let spectator = params
        .access_token
        .as_deref()
        .is_some_and(|token| roles(token).contains(&Role::Spectate));

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
        .insert_resource(sfx_catalog())
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
        ))
        .add_plugins((
            systems::actor::ActorPlugin,
            systems::area::AreaPlugin,
            systems::combat::CombatPlugin,
            systems::item::ItemsPlugin,
            systems::view::ViewPlugin,
            systems::session::SessionPlugin,
            systems::input::InputPlugin,
            systems::debug::DebugPlugin,
            systems::testing::TestingPlugin,
            systems::hud::scenes::ScenesPlugin,
            systems::hud::connection::ConnectionPlugin,
            systems::hud::HudPlugin,
            systems::hud::fps::FpsPlugin,
        ));
    ui::theme::set_theme(ui::themes::dark::THEME);
    app.run();
}

/// Builds the core audio mixer's sound catalogue from `world`'s sfx table — the one place the client
/// bridges game content into the game-agnostic mixer.
fn sfx_catalog() -> core::audio::SfxCatalog {
    core::audio::SfxCatalog(
        world::systems::sfx::sfx_table()
            .iter()
            .map(|def| core::audio::SfxSpec {
                id: def.id.0.clone(),
                path: def.src.clone(),
                volume: def.volume.range(),
                pitch: def.pitch.range(),
            })
            .collect(),
    )
}

/// Decodes the player's roles from the access token's JWT claims — client-side and presentation-only
/// (it just decides whether to offer the spectate choice); the server remains the source of truth.
fn roles(access_token: &str) -> Vec<Role> {
    let Some(payload) = access_token.split('.').nth(1) else {
        return Vec::new();
    };
    let Ok(bytes) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload) else {
        return Vec::new();
    };
    let Ok(claims) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Vec::new();
    };
    claims["realm_access"]["roles"]
        .as_array()
        .map(|roles| {
            roles
                .iter()
                .filter_map(|role| role.as_str().and_then(Role::parse))
                .collect()
        })
        .unwrap_or_default()
}
