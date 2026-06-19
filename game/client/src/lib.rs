use std::path::{Path, PathBuf};

use bevy::prelude::*;

pub mod auth;
pub mod cursor;
pub mod debug;
pub mod fps;
pub mod hud;
pub mod input;
pub mod net;
pub mod render;
pub mod screens;
pub mod sfx;
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

/// The directory the executable lives in — the bundle root for a distributed client. A relative
/// `RIFT_ASSETS_DIR` is anchored here, and the `.env` shipped beside the binary is loaded from here.
fn bundle_dir() -> Option<PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.to_owned())
}

/// Resolves `RIFT_ASSETS_DIR` to an absolute path: a relative value anchors to the bundle dir (so a
/// distributed client finds the assets shipped beside it), an absolute one is used as-is. It must be
/// absolute because Bevy resolves its asset path against the executable's directory, not the cwd.
fn assets_root(bundle: Option<&Path>) -> PathBuf {
    let raw =
        PathBuf::from(std::env::var_os("RIFT_ASSETS_DIR").expect("RIFT_ASSETS_DIR must be set"));
    let absolute = match bundle {
        Some(dir) if raw.is_relative() => dir.join(raw),
        _ => raw,
    };
    std::fs::canonicalize(&absolute).unwrap_or(absolute)
}

pub fn run() -> AppExit {
    let bundle = bundle_dir();
    // Load the `.env` shipped beside the executable before reading any config, so a distributed
    // client is configured by the file next to it. Already-set vars win, so dev and the e2e (which
    // export their own) are untouched, and an absent file is fine.
    if let Some(dir) = &bundle {
        let _ = dotenvy::from_path(dir.join(".env"));
    }
    world::assets::init(assets_root(bundle.as_deref()));
    let config: auth::ClientConfig = envy::prefixed("RIFT_CLIENT_")
        .from_env()
        .expect("RIFT_CLIENT_* environment");

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
                file_path: world::assets::root().to_string_lossy().into_owned(),
                watch_for_changes_override: cfg!(feature = "hotpatch").then_some(true),
                ..default()
            }),
    )
    .insert_resource(config)
    .init_state::<Screen>()
    .add_plugins((
        bevy_view::ViewPlugin,
        ui::UiPlugin,
        net::NetPlugin,
        auth::AuthPlugin,
        render::RenderPlugin,
        input::InputPlugin,
        cursor::CursorPlugin,
        debug::DebugPlugin,
        sfx::SfxPlugin,
        screens::ScreensPlugin,
        hud::HudPlugin,
        fps::FpsPlugin,
    ));
    app.run()
}
