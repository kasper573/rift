//! `render <map> <out.png>` — rasterizes a Tiled map to a PNG with an offscreen Bevy app, so maps can
//! be previewed without running the game. `<map>` is a `.tmx` path, or a bare name resolved against
//! `assets/maps/<name>.tmx`. Headless: a Vulkan render device, no window, a camera drawing the whole
//! map to an image target that gets screenshotted once the GPU has settled.

use std::path::{Path, PathBuf};

use bevy::camera::{RenderTarget, ScalingMode};
use bevy::image::ImageSampler;
use bevy::log::{Level, LogPlugin};
use bevy::prelude::*;
use bevy::render::RenderPlugin;
use bevy::render::render_resource::TextureFormat;
use bevy::render::settings::{Backends, RenderCreation, WgpuSettings};
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk};
use bevy::window::ExitCondition;

use bevy_tiled::{Files, TileAnimationPlugin, spawn_map};

const SETTLE_FRAMES: u32 = 12;
const MAX_FRAMES: u32 = 2000;

#[derive(Resource)]
struct Map(tiled::Map);

#[derive(Resource)]
struct Job {
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
    let (map_arg, out) = parse_args();
    let map = load_map(&map_arg);
    let width = (map.width * map.tile_width).max(1);
    let height = (map.height * map.tile_height).max(1);

    let mut app = App::new();
    app.add_plugins(
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
    .add_plugins(TileAnimationPlugin)
    .insert_resource(ClearColor(Color::srgb(0.05, 0.05, 0.08)))
    .init_resource::<Done>()
    .insert_resource(Map(map))
    .insert_resource(Job {
        out,
        width,
        height,
        settled: 0,
        requested: false,
    })
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

fn load_map(arg: &str) -> tiled::Map {
    let path = if Path::new(arg).is_file() {
        PathBuf::from(arg)
    } else {
        PathBuf::from(format!("assets/maps/{arg}.tmx"))
    };
    tiled::Loader::with_reader(|path: &Path| std::fs::File::open(path))
        .load_tmx_map(&path)
        .unwrap_or_else(|error| {
            eprintln!("render: cannot load map '{}': {error}", path.display());
            std::process::exit(2);
        })
}

fn setup(mut commands: Commands, map: Res<Map>, job: Res<Job>, mut images: ResMut<Assets<Image>>) {
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
        // Tiles are centred in [0, width] x [-height, 0]; centre the camera on that box.
        Transform::from_xyz(job.width as f32 / 2.0, -(job.height as f32) / 2.0, 0.0),
        Msaa::Off,
    ));

    spawn_map(&mut commands, &mut images, &map.0, &mut Files::default());
}

fn capture_when_ready(mut job: ResMut<Job>, target: Res<Target>, mut commands: Commands) {
    if job.requested {
        return;
    }
    job.settled += 1;
    if job.settled < SETTLE_FRAMES {
        return;
    }
    job.requested = true;
    commands
        .spawn(Screenshot::image(target.0.clone()))
        .observe(save_to_disk(job.out.clone()))
        .observe(|_: On<ScreenshotCaptured>, mut done: ResMut<Done>| done.0 = true);
}

fn parse_args() -> (String, PathBuf) {
    let mut args = std::env::args().skip(1);
    let map = args.next().unwrap_or_else(|| {
        eprintln!("usage: render <map> <out.png>");
        std::process::exit(2);
    });
    let out = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("{map}.png")));
    (map, out)
}
