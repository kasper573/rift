//! `render <map> [out.png]` — rasterizes a whole map to an image so you can preview it without
//! launching the game. It runs the real [`bevy_tiled`] sprite renderer in a windowless Bevy app that
//! draws the map to an offscreen texture sized to the map, then screenshots that texture to a PNG.

use std::path::PathBuf;

use bevy::asset::AssetApp;
use bevy::asset::io::memory::{Dir, MemoryAssetReader};
use bevy::asset::io::{AssetSourceBuilder, AssetSourceId, ErasedAssetReader};
use bevy::camera::{RenderTarget, ScalingMode};
use bevy::image::ImageSampler;
use bevy::log::{Level, LogPlugin};
use bevy::prelude::*;
use bevy::render::RenderPlugin;
use bevy::render::render_resource::TextureFormat;
use bevy::render::settings::{Backends, RenderCreation, WgpuSettings};
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk};
use bevy::window::ExitCondition;
use bevy_tiled::{AreaTile, TILE, spawn_area};
use world::systems::area;

/// Once every tile sprite has been on screen for this many frames its texture is uploaded, so the
/// screenshot captures the finished map rather than load-in-progress fallbacks.
const SETTLE_FRAMES: u32 = 5;
/// Hard cap so a stalled GPU or missing asset fails loudly instead of hanging.
const MAX_FRAMES: u32 = 2000;

#[derive(Resource)]
struct Job {
    area: usize,
    out: PathBuf,
    width: u32,
    height: u32,
    settled: u32,
    requested: bool,
}

#[derive(Resource, Default)]
struct Done(bool);

#[derive(Resource)]
struct Target(Handle<Image>);

fn main() {
    let (map, out) = parse_args();
    let job = resolve(&map, out);

    let mut app = App::new();
    app.register_asset_source(AssetSourceId::Default, embedded_assets())
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: ExitCondition::DontExit,
                    close_when_requested: false,
                    ..default()
                })
                .set(ImagePlugin::default_nearest())
                .set(RenderPlugin {
                    render_creation: RenderCreation::Automatic(Box::new(WgpuSettings {
                        // Vulkan renders offscreen without a display; the GL backend would need one.
                        backends: Some(Backends::VULKAN),
                        ..default()
                    })),
                    ..default()
                })
                .set(LogPlugin {
                    level: Level::WARN,
                    filter: "wgpu=error,naga=error,bevy_render=warn".to_owned(),
                    ..default()
                }),
        )
        .insert_resource(ClearColor(Color::srgb(0.05, 0.05, 0.08)))
        .init_resource::<Done>()
        .insert_resource(job)
        .add_systems(Startup, setup)
        .add_systems(Update, capture_when_ready);

    app.finish();
    app.cleanup();
    for _ in 0..MAX_FRAMES {
        app.update();
        if app.world().resource::<Done>().0 {
            return;
        }
    }
    eprintln!("render: timed out before the map finished drawing");
    std::process::exit(1);
}

fn setup(
    mut commands: Commands,
    assets: Res<AssetServer>,
    job: Res<Job>,
    mut images: ResMut<Assets<Image>>,
) {
    let mut texture =
        Image::new_target_texture(job.width, job.height, TextureFormat::Rgba8UnormSrgb, None);
    texture.sampler = ImageSampler::nearest();
    let target = images.add(texture);
    commands.insert_resource(Target(target.clone()));

    commands.spawn((
        Camera2d,
        Camera {
            order: 0,
            ..default()
        },
        RenderTarget::Image(target.into()),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::Fixed {
                width: job.width as f32,
                height: job.height as f32,
            },
            ..OrthographicProjection::default_2d()
        }),
        // The map spans world x in [0, width] and y in [-height, 0]; centre the camera on it.
        Transform::from_xyz(job.width as f32 / 2.0, -(job.height as f32) / 2.0, 0.0),
        Msaa::Off,
    ));

    spawn_area(&mut commands, &assets, &area::areas()[job.area]);
}

fn capture_when_ready(
    mut job: ResMut<Job>,
    tiles: Query<&Sprite, With<AreaTile>>,
    assets: Res<AssetServer>,
    target: Res<Target>,
    mut commands: Commands,
) {
    if job.requested {
        return;
    }
    let loaded = !tiles.is_empty()
        && tiles
            .iter()
            .all(|sprite| assets.is_loaded_with_dependencies(sprite.image.id()));
    job.settled = if loaded { job.settled + 1 } else { 0 };
    if job.settled < SETTLE_FRAMES {
        return;
    }
    job.requested = true;
    commands
        .spawn(Screenshot::image(target.0.clone()))
        .observe(save_to_disk(job.out.clone()))
        .observe(|_: On<ScreenshotCaptured>, mut done: ResMut<Done>| done.0 = true);
}

fn resolve(map: &str, out: PathBuf) -> Job {
    let area = area::defs()
        .iter()
        .position(|def| def.id == map || def.map.0 == map)
        .unwrap_or_else(|| {
            let available: Vec<&str> = area::defs().iter().map(|def| def.id.as_str()).collect();
            eprintln!(
                "render: unknown map '{map}'. available: {}",
                available.join(", ")
            );
            std::process::exit(2);
        });
    let size = area::areas()[area].size;
    Job {
        area,
        out,
        width: (size.width * TILE.0).ceil() as u32,
        height: (size.height * TILE.0).ceil() as u32,
        settled: 0,
        requested: false,
    }
}

fn parse_args() -> (String, PathBuf) {
    let mut args = std::env::args().skip(1);
    let map = args.next().unwrap_or_else(|| {
        eprintln!("usage: render <map> [out.png]");
        std::process::exit(2);
    });
    let out = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("{map}.png")));
    (map, out)
}

/// A Bevy asset source backed by the same embedded asset tree the game ships, so the tool needs no
/// asset directory on disk — it reads tilesets straight from the `world` binary embed.
fn embedded_assets() -> AssetSourceBuilder {
    let root = Dir::new(PathBuf::new());
    fill(&root, world::core::assets::dir());
    AssetSourceBuilder::new(move || {
        Box::new(MemoryAssetReader { root: root.clone() }) as Box<dyn ErasedAssetReader>
    })
}

fn fill(dir: &Dir, embedded: &'static include_dir::Dir<'static>) {
    for file in embedded.files() {
        dir.insert_asset(file.path(), file.contents());
    }
    for sub in embedded.dirs() {
        fill(dir, sub);
    }
}
