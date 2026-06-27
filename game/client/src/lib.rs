use base64::Engine;
use bevy::asset::AssetApp;
use bevy::asset::io::AssetSourceId;
use bevy::prelude::*;
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
        .add_plugins((
            ui::UiPlugin,
            core::net::NetPlugin,
            core::render::RenderPlugin,
            core::audio::SfxPlugin,
        ))
        .add_plugins((
            systems::scene::ScenePlugin { spectator },
            systems::actor::ActorPlugin,
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

fn sfx_catalog() -> core::audio::SfxCatalog {
    core::audio::SfxCatalog(
        world::data::sfx::TABLE
            .iter()
            .map(|(id, def)| core::audio::SfxSpec {
                id: id.name().to_owned(),
                path: def.src.0.to_owned(),
                volume: def.volume.range(),
                pitch: def.pitch.range(),
            })
            .collect(),
    )
}

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
                .filter_map(|role| role.as_str().and_then(|name| name.parse::<Role>().ok()))
                .collect()
        })
        .unwrap_or_default()
}
