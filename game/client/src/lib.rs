use bevy::prelude::*;

pub mod auth;
pub mod cursor;
pub mod debug;
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

/// Loads the `.env` shipped beside the executable into the environment before anything reads it, so a
/// distributed client is configured by the file next to it. Already-set vars win, so dev and the e2e
/// (which export their own) are untouched, and an absent file is fine. A relative `RIFT_ASSETS_DIR`
/// in that file is anchored to the executable's directory — the bundle root.
fn load_bundle_env() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(dir) = exe.parent() else {
        return;
    };
    let _ = dotenvy::from_path(dir.join(".env"));
    if let Some(assets) = std::env::var_os("RIFT_ASSETS_DIR") {
        // SAFETY: called first in run(), before any thread or asset system starts.
        unsafe {
            std::env::set_var("RIFT_ASSETS_DIR", dir.join(assets));
        }
    }
}

pub fn run() -> AppExit {
    load_bundle_env();
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(bevy::log::LogPlugin {
                // symphonia narrates every wav metadata chunk it skips at info.
                filter: format!("{},symphonia=warn", bevy::log::DEFAULT_FILTER),
                ..default()
            })
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
