use bevy::asset::AssetApp;
use bevy::asset::io::AssetSourceId;
use bevy::prelude::*;
use wasm_bindgen::prelude::wasm_bindgen;
use world::Role;

pub mod assets;
pub mod auth;
pub mod cursor;
pub mod debug;
pub mod drag;
pub mod fps;
pub mod hud;
pub mod input;
pub mod net;
pub mod render;
pub mod replicon_renet;
pub mod screen;
pub mod screens;
pub mod sfx;
pub mod start;
pub mod user_settings;
pub mod view;

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

/// The page's loader calls this after `init()`. Reads the start params the website wrote onto
/// `#glcanvas`, builds the session from the access token, and runs the Bevy app on that canvas.
#[wasm_bindgen]
pub fn run() {
    console_error_panic_hook::set_once();
    let params = start::read();
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
                    primary_window: Some(Window {
                        title: "rift mmo".to_owned(),
                        canvas: Some("#glcanvas".to_owned()),
                        ..default()
                    }),
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
            drag::DragPlugin,
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
