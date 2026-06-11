//! The Bevy client: signs in through the browser, opens a netcode session to the `world` server,
//! and renders the replicated simulation with a HUD. Composed from one plugin per concern.

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
pub mod smoke;
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

/// The assets directory as an absolute path (`RIFT_ASSETS` or `assets`, resolved against the
/// working directory).
fn assets_root() -> String {
    let root = world::assets::root();
    std::fs::canonicalize(&root)
        .unwrap_or(root)
        .to_string_lossy()
        .into_owned()
}

pub fn run() -> AppExit {
    let mut app = App::new();
    // Dist builds serve assets from the binary; dev and servers read them from disk.
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
                // Hot-reload assets from disk in the dev/hotpatch loops (both enable file_watcher).
                watch_for_changes_override: cfg!(any(feature = "dev", feature = "hotpatch"))
                    .then_some(true),
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
        smoke::SmokePlugin,
    ));
    app.run()
}
