use bevy::prelude::*;

pub mod auth;
pub mod cursor;
pub mod debug;
#[cfg(feature = "dist")]
pub mod embedded;
pub mod input;
pub mod net;
pub mod render;
pub mod screens;
pub mod sfx;
pub mod ui;
pub mod user_settings;
pub mod view;
pub mod web;

/// The top-level flow: sign in through the browser, optionally choose a mode, then play.
#[derive(States, Default, Debug, Clone, PartialEq, Eq, Hash)]
pub enum Screen {
    #[default]
    SigningIn,
    SignInFailed,
    ChooseMode,
    Playing,
}

fn assets_root() -> String {
    let root = world::assets::root();
    std::fs::canonicalize(&root)
        .unwrap_or(root)
        .to_string_lossy()
        .into_owned()
}

pub fn run() -> AppExit {
    let mut app = App::new();
    #[cfg(feature = "dist")]
    embedded::register(&mut app);
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "rift mmo".to_owned(),
                    resolution: UVec2::new(1152, 864).into(),
                    ..default()
                }),
                ..default()
            })
            .set(ImagePlugin::default_nearest())
            .set(AssetPlugin {
                // Absolute, because Bevy resolves a relative path against the executable's
                // directory, not the working directory the assets live under.
                file_path: assets_root(),
                watch_for_changes_override: cfg!(feature = "hotpatch").then_some(true),
                ..default()
            }),
    )
    .init_state::<Screen>()
    .add_plugins((
        net::NetPlugin,
        auth::AuthPlugin,
        render::RenderPlugin,
        input::InputPlugin,
        cursor::CursorPlugin,
        debug::DebugPlugin,
        sfx::SfxPlugin,
        screens::ScreensPlugin,
        ui::HudPlugin,
    ));
    app.run()
}
