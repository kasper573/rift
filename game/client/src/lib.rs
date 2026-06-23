use bevy::asset::AssetApp;
use bevy::asset::io::AssetSourceId;
use bevy::prelude::*;
use world::account::Role;

pub mod assets;
pub mod debug;
pub mod hud;
pub mod input;
pub mod net;
pub mod platform;
pub mod render;
pub mod session;
pub mod sfx;
pub mod testing;

#[derive(States, Default, Debug, Clone, PartialEq, Eq, Hash)]
pub enum GameScene {
    /// Spectators choose whether to play or watch; players skip straight to [`GameScene::Playing`].
    #[default]
    ChooseMode,
    Playing,
}

/// Inserts an already-built [`Component`] value into a `bsn!` scene — the bridge for our builder-style
/// helpers and `FromTemplate`/`Arc`-backed components (e.g. `ImageNode`, `OnSettle`) that `bsn!`'s
/// `template_value` (which needs plain `Default + Clone`) can't take.
pub(crate) fn component<C: Component + Clone>(value: C) -> impl bevy::scene::Scene {
    bevy::ecs::template::FnTemplate(move |_: &mut bevy::ecs::template::TemplateContext| {
        Ok(value.clone())
    })
}

/// Builds and runs the Bevy app: reads the boot params from the platform, builds the session from the
/// access token, and starts the app. The platform's entry point calls this.
pub fn boot() {
    let params = platform::read_start_params();
    let session = params
        .access_token
        .as_deref()
        .map(net::auth::Session::from_access_token);
    let spectator = session
        .as_ref()
        .is_some_and(|session| session.roles.contains(&Role::Spectate));

    let mut app = App::new();
    app.register_asset_source(AssetSourceId::Default, assets::embedded_source())
        .add_plugins(
            DefaultPlugins
                .set(bevy::log::LogPlugin {
                    filter: format!("{},symphonia=warn", bevy::log::DEFAULT_FILTER),
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: Some(platform::primary_window()),
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
            net::NetPlugin,
            render::RenderPlugin,
            input::InputPlugin,
            debug::DebugPlugin,
            sfx::SfxPlugin,
            hud::scenes::ScenesPlugin,
            hud::connection::ConnectionPlugin,
            hud::HudPlugin,
            hud::fps::FpsPlugin,
            testing::TestingPlugin,
        ));
    ui::theme::set_theme(ui::themes::dark::THEME);
    app.run();
}
