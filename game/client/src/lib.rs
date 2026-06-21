use std::path::{Path, PathBuf};

use bevy::prelude::*;

pub mod auth;
pub mod cursor;
pub mod debug;
pub mod drag;
pub mod fps;
pub mod hud;
pub mod input;
pub mod net;
pub mod render;
pub mod screen;
pub mod screens;
pub mod sfx;
pub mod user_settings;
pub mod view;
pub mod web;

#[derive(States, Default, Debug, Clone, PartialEq, Eq, Hash)]
pub enum Screen {
    #[default]
    SigningIn,
    SignInFailed,
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

fn bundle_dir() -> Option<PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.to_owned())
}

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
        ui::UiPlugin,
        drag::DragPlugin,
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
    ui::theme::set_theme(ui::themes::dark::THEME);
    app.run()
}
