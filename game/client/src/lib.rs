use bevy::asset::AssetApp;
use bevy::asset::io::AssetSourceId;
use bevy::prelude::*;
use world::Role;

pub mod assets;
pub mod auth;
pub mod cursor;
pub mod debug;
pub mod fps;
pub mod gestures;
pub mod hud;
pub mod input;
pub mod net;
pub mod platform;
pub mod render;
pub mod replicon_renet;
pub mod screen;
pub mod screens;
pub mod sfx;
pub mod user_settings;

#[derive(States, Default, Debug, Clone, PartialEq, Eq, Hash)]
pub enum Screen {
    /// Spectators choose whether to play or watch; players skip straight to [`Screen::Playing`].
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
        .map(auth::Session::from_access_token);
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
            Screen::ChooseMode
        } else {
            Screen::Playing
        })
        .add_plugins((
            ui::UiPlugin,
            net::NetPlugin,
            render::RenderPlugin,
            input::InputPlugin,
            cursor::CursorPlugin,
            debug::DebugPlugin,
            sfx::SfxPlugin,
            screens::ScreensPlugin,
            hud::HudPlugin,
            fps::FpsPlugin,
        ));
    ui::theme::set_theme(ui::themes::dark::THEME);
    app.run();
}
