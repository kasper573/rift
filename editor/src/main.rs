use std::net::SocketAddr;
use std::path::PathBuf;

use bevy::asset::AssetApp;
use bevy::asset::io::AssetSourceId;
use bevy::prelude::*;
use bevy::window::{PresentMode, WindowResolution};
use clap::error::ErrorKind;
use clap::{CommandFactory, Parser};
use editor::app::{EditorLaunch, EditorPlugin};
use editor::scenes::SceneStep;
use engine::core::assets::SyncedAssetReader;
use engine::core::audio::playback::SfxPlugin;
use engine::core::babble::BabblePlugin;
use engine::core::math::Rng;
use engine::core::render::world::WorldScenePlugin;
use engine::core::text::StyledTextPlugin;
use engine::core::vfx::render::VfxMaterialsPlugin;
use engine::systems::area::scenery::SceneryPlugin;
use engine::systems::launch;
use strum::VariantNames;

/// The Rift editor.
#[derive(Parser)]
struct Options {
    /// Opens this game folder right away.
    folder: Option<PathBuf>,
    #[arg(long, value_name = "JSON", long_help = scene_help())]
    scene: Option<String>,
    /// Keeps settings, journal and import records here instead of in the user's folder.
    #[arg(long, value_name = "PATH")]
    user_folder: Option<PathBuf>,
    /// Writes a line of JSON per frame: how long it took, the active tab, what the world holds and
    /// how its scenery stands.
    #[arg(long, value_name = "PATH")]
    frame_log: Option<PathBuf>,
    /// Answers the probe's driver listening at this address, as a client's probe does.
    #[arg(long, value_name = "ADDRESS")]
    probe: Option<SocketAddr>,
    /// Prints the build and exits.
    #[arg(long)]
    build_info: bool,
}

fn main() {
    let options = Options::parse();
    if options.build_info {
        println!("{}", launch::build_info_line());
        return;
    }
    let scene = options.scene.map(|json| {
        serde_json::from_str(&json).unwrap_or_else(|error| {
            Options::command()
                .error(ErrorKind::ValueValidation, format!("--scene: {error}"))
                .exit()
        })
    });
    let launch = EditorLaunch {
        folder: options.folder,
        scene,
        user: options.user_folder,
        frame_log: options.frame_log,
        probe: options.probe,
    };
    let files = SyncedAssetReader::default();
    let mut app = App::new();
    app.insert_resource(files.clone())
        .insert_resource(Rng::from_entropy());
    AssetApp::register_asset_source(&mut app, AssetSourceId::Default, files.source())
        .add_plugins(
            DefaultPlugins
                .set(bevy::log::LogPlugin {
                    filter: format!("{},symphonia=warn", bevy::log::DEFAULT_FILTER),
                    custom_layer: engine::systems::probe::timing_layer,
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Rift editor".to_owned(),
                        resolution: WindowResolution::new(1600, 1000),
                        present_mode: PresentMode::AutoNoVsync,
                        ..default()
                    }),
                    ..default()
                })
                .set(ImagePlugin::default_nearest()),
        )
        .add_plugins((SfxPlugin, StyledTextPlugin, BabblePlugin))
        .add_plugins((WorldScenePlugin, VfxMaterialsPlugin, SceneryPlugin))
        .add_plugins(EditorPlugin { launch })
        .run();
}

fn scene_help() -> String {
    format!(
        "Opens the folder on a state, for recordings and tests: a JSON array of steps, each an object \
         keyed by its step name, e.g. [{{\"area\":\"island\"}},{{\"model\":{{\"open\":\"walk\"}}}}]. \
         The steps: {}. A step that opens a view, as model does, takes that view's own steps.",
        SceneStep::VARIANTS.join(", ")
    )
}
