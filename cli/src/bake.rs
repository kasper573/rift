//! Baking a game's maps off screen, as the editor bakes them, into the bundles of its areas.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use bevy::window::ExitCondition;
use engine::core::audio::playback::Listener;
use engine::core::content::{Content, FilesystemFolder, FolderReader, LoadPurpose};
use engine::core::jobs::Handoff;
use engine::core::math::Rng;
use engine::core::os::Profile;
use engine::core::render::video::VideoPreferences;
use engine::core::render::world::WorldScenePlugin;
use engine::core::vfx::render::VfxMaterialsPlugin;
use engine::systems::area::AreaDef;
use engine::systems::area::rain::{RainField, Shelter};
use engine::systems::area::scenery::SceneryPlugin;
use engine::systems::area_map::bake::{self, BakedMaps, MapBakePlugin, MapBakeProgress};
use engine::systems::{GameSettings, registry};

/// Bakes the maps of every area of the game in `folder` into its bundles, drawing again only
/// the squares whose ground, props or style changed since they were baked, keeping what the
/// renderer works out once (the models' impostors) under `profile`, or under a folder of this
/// run's alone.
pub fn maps(folder: &Path, profile: Option<&Path>) {
    let folder = std::fs::canonicalize(folder).expect("a game folder");
    let own_profile = tempfile::tempdir().expect("a scratch profile");
    let profile =
        Profile(profile.map_or_else(|| own_profile.path().to_path_buf(), Path::to_path_buf));
    let content = crate::folder::load(&folder, "the game did not load", LoadPurpose::Play)
        .unwrap_or_else(|report| panic!("{report}"));
    let mut areas = Some(content.ids::<AreaDef>().collect::<Vec<_>>());
    let baking = content.clone();
    let written = folder.clone();
    let mut told = Instant::now();
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: ExitCondition::DontExit,
                    ..default()
                })
                .set(AssetPlugin {
                    file_path: folder.display().to_string(),
                    ..default()
                })
                .disable::<bevy::winit::WinitPlugin>(),
        )
        .add_plugins(ScheduleRunnerPlugin::run_loop(FRAME))
        .insert_resource(Rng::from_entropy())
        .insert_resource(Handoff::NextFrame)
        .insert_resource(profile)
        .add_plugins((
            WorldScenePlugin,
            VfxMaterialsPlugin,
            SceneryPlugin,
            MapBakePlugin,
        ))
        .insert_resource(content.built::<GameSettings>().look)
        .insert_resource(content.clone())
        .insert_resource(VideoPreferences::default())
        .init_resource::<RainField>()
        .init_resource::<Shelter>()
        .init_resource::<Listener>()
        .add_systems(Startup, move |world: &mut World| {
            let areas = areas.take().expect("startup runs once");
            bake::begin(world, baking.clone(), areas);
        })
        .add_systems(Update, move |world: &mut World| {
            if let Some(progress) = bake::progress(world)
                && told.elapsed() >= TOLD_EVERY
            {
                told = Instant::now();
                match progress {
                    MapBakeProgress::Planning { come, total } => {
                        println!("finding the props' plans from above: {come} of {total}");
                    }
                    MapBakeProgress::Drawing { drawn, total } => {
                        println!("drawing the squares: {drawn} of {total}");
                    }
                }
            }
            if let Some(baked) = bake::take_baked(world) {
                let content = world.resource::<Content>().clone();
                write(&written, &content, &baked);
                world.write_message(AppExit::Success);
            }
        })
        .run();
}

/// Writes the maps `baked` of the game in `folder`, as `content` names its areas: only the
/// files of each that changed.
fn write(folder: &Path, content: &Content, baked: &BakedMaps) {
    let reader = FolderReader::new(registry(), Arc::new(FilesystemFolder(folder.to_path_buf())));
    for area in &baked.areas {
        let path = area.area.path(content);
        let mut row: AreaDef = reader
            .read(path)
            .unwrap_or_else(|errors| panic!("{path} does not read as an area: {errors:?}"));
        row.map = area.map.clone();
        let written = reader
            .write_only(path, &row, area.changes(path))
            .unwrap_or_else(|error| panic!("{path}'s map does not write: {error}"));
        for (file, bytes) in &written.files {
            let file = folder.join(file);
            if let Some(parent) = file.parent() {
                std::fs::create_dir_all(parent).expect("the folder of a map's picture");
            }
            std::fs::write(file, bytes).expect("a file of a map written");
        }
        for file in &written.removed {
            std::fs::remove_file(folder.join(file)).expect("a map's picture removed");
        }
        println!(
            "{path}: drew {} of its map's {} squares, {} were up to date",
            area.drawn(),
            area.map.squares.len(),
            area.kept.len()
        );
    }
    println!(
        "baked in {:.1?}, of which finding the props' plans took {:.1?}",
        baked.took, baked.planned
    );
}

/// How often the bake is moved on while it waits on the GPU and its other cores.
const FRAME: Duration = Duration::from_millis(5);
/// How often the bake tells how far it has come.
const TOLD_EVERY: Duration = Duration::from_secs(2);
