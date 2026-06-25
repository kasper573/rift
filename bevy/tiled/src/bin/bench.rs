//! `bench [map]` — measures the tiled renderer phase by phase so a change can be compared before and
//! after. `[map]` is a `.tmx` path or a bare name under `assets/maps/<name>.tmx` (default `island`,
//! the largest). It reports:
//!   - **load**: parsing the `.tmx` and its tilesets off disk.
//!   - **spawn**: turning the map into entities (meshes merged by tileset/depth/animation) + the count.
//!   - **render fps**: frames rendered in a fixed budget, timed from the first frame so load/spawn are
//!     excluded, with the loop driving `app.update()` flat out (no window, so no vsync caps the rate).
//!
//! Two camera scenarios separate the costs: a *game view* (zoomed, most tiles culled — the per-frame
//! CPU cost the running game actually pays) and a *whole map* (everything on screen — raw draw cost).
//!
//! Headless with a real render device, no window. `WGPU_BACKEND=gl` forces software GL to mirror CI.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bevy::camera::{RenderTarget, ScalingMode};
use bevy::ecs::system::RunSystemOnce;
use bevy::image::ImageSampler;
use bevy::log::{Level, LogPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::window::ExitCondition;

use bevy_tiled::{Files, MapTile, TILE, TileAnimationPlugin, spawn_map};

const BUDGET: Duration = Duration::from_secs(5);
// The game draws art at its native size and zooms the camera by this so a slice of the map fills a
// desktop viewport — replicated here so the "game view" scenario culls and pays per-frame like the game.
const TILE_SCREEN: f32 = 96.0;
const SCALE: f32 = TILE_SCREEN / TILE;
const VIEW_W: u32 = 1280;
const VIEW_H: u32 = 800;

#[derive(Resource)]
struct MapRes(tiled::Map);

#[derive(Component)]
struct BenchCamera;

fn main() {
    let map_name = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "island".to_owned());

    let started = Instant::now();
    let map = load_map(&map_name);
    let load_ms = ms(started.elapsed());
    let (map_w, map_h) = (map.width, map.height);

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
            .set(LogPlugin {
                level: Level::WARN,
                filter: "wgpu=error,naga=error,bevy_render=warn".to_owned(),
                ..default()
            }),
    )
    .add_plugins(TileAnimationPlugin)
    .insert_resource(MapRes(map))
    .add_systems(Startup, setup);

    app.finish();
    app.cleanup();
    // Bring up the render device and the (empty) camera before timing anything tile-related.
    app.update();

    // Spawn phase: build every tile entity. run_system_once flushes the queued commands, so the timing
    // covers the entity creation itself, not just queueing it.
    let started = Instant::now();
    app.world_mut()
        .run_system_once(spawn_tiles)
        .expect("spawn tiles");
    let spawn_ms = ms(started.elapsed());

    let tiles = app
        .world_mut()
        .query_filtered::<(), With<MapTile>>()
        .iter(app.world())
        .count();

    let game_view = render_fps(&mut app);

    aim_camera_at_whole_map(&mut app, map_w, map_h);
    let whole_map = render_fps(&mut app);

    println!("[tiled-bench] map={map_name} ({map_w}x{map_h}) tiles={tiles}");
    println!("[tiled-bench] load:  {load_ms:.2}ms");
    println!("[tiled-bench] spawn: {spawn_ms:.2}ms ({tiles} entities)");
    report("game view ", &game_view);
    report("whole map ", &whole_map);
    println!(
        "[tiled-bench] RESULT {map_name},{tiles},{load_ms:.3},{spawn_ms:.3},{:.1},{:.1}",
        game_view.fps, whole_map.fps
    );
}

struct Fps {
    frames: u64,
    fps: f64,
    avg_ms: f64,
    first_ms: f64,
    max_ms: f64,
}

/// Drives `app.update()` flat out for [`BUDGET`], timing from the first frame, and reports throughput.
/// Calls out the first frame (the one-off pipeline compile + initial extract of every tile) and the
/// worst frame separately, since those one-off stalls — not the average — are what trips a net timeout.
fn render_fps(app: &mut App) -> Fps {
    let start = Instant::now();
    let mut frames = 0u64;
    let mut first_ms = 0.0;
    let mut max_ms: f64 = 0.0;
    while start.elapsed() < BUDGET {
        let frame_started = Instant::now();
        app.update();
        let frame_ms = ms(frame_started.elapsed());
        if frames == 0 {
            first_ms = frame_ms;
        }
        max_ms = max_ms.max(frame_ms);
        frames += 1;
    }
    let secs = start.elapsed().as_secs_f64();
    Fps {
        frames,
        fps: frames as f64 / secs,
        avg_ms: secs * 1000.0 / frames as f64,
        first_ms,
        max_ms,
    }
}

fn report(label: &str, fps: &Fps) {
    println!(
        "[tiled-bench] render fps ({label}): {} frames → {:.1} fps (avg {:.3}ms, first {:.2}ms, max {:.2}ms)",
        fps.frames, fps.fps, fps.avg_ms, fps.first_ms, fps.max_ms,
    );
}

fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let mut texture =
        Image::new_target_texture(VIEW_W, VIEW_H, TextureFormat::Rgba8UnormSrgb, None);
    texture.sampler = ImageSampler::nearest();
    let target = images.add(texture);

    commands.spawn((
        Camera2d,
        Camera {
            order: 0,
            ..default()
        },
        RenderTarget::Image(target.into()),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::Fixed {
                width: VIEW_W as f32 / SCALE,
                height: VIEW_H as f32 / SCALE,
            },
            ..OrthographicProjection::default_2d()
        }),
        Transform::default(),
        Msaa::Off,
        BenchCamera,
    ));
}

fn spawn_tiles(
    map: Res<MapRes>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut commands: Commands,
) {
    spawn_map(
        &mut commands,
        &mut images,
        &mut meshes,
        &mut materials,
        &map.0,
        &mut Files::default(),
        Vec2::ZERO,
    );
}

/// Reframes the camera to show the entire map, so the next scenario draws every tile rather than culling.
fn aim_camera_at_whole_map(app: &mut App, map_w: u32, map_h: u32) {
    let world = app.world_mut();
    let mut cameras =
        world.query_filtered::<(&mut Projection, &mut Transform), With<BenchCamera>>();
    for (mut projection, mut transform) in cameras.iter_mut(world) {
        if let Projection::Orthographic(ortho) = projection.as_mut() {
            ortho.scaling_mode = ScalingMode::Fixed {
                width: map_w as f32 * TILE,
                height: map_h as f32 * TILE,
            };
        }
        transform.translation =
            Vec3::new(map_w as f32 * TILE / 2.0, -(map_h as f32) * TILE / 2.0, 0.0);
    }
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
            eprintln!("bench: cannot load map '{}': {error}", path.display());
            std::process::exit(2);
        })
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}
