use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};

use bevy::asset::{AssetPlugin, LoadState};
use bevy::camera::{RenderTarget, ScalingMode};
use bevy::image::ImageSampler;
use bevy::log::{Level, LogPlugin};
use bevy::prelude::*;
use bevy::render::RenderPlugin;
use bevy::render::render_resource::TextureFormat;
use bevy::render::settings::{Backends, RenderCreation, WgpuSettings};
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::window::ExitCondition;

use game::core::assets::{AssetService, FilesystemSource};
use game::core::math::{Direction, Size, WorldPx};
use game::core::time::{PlaybackRate, Seconds};
use game::data::model;
use game::systems::actor::render::{ACTOR_ANCHOR, draw_actor_frame};
use game::systems::actor::{Action, ActorModel, build_model};

const FPS: f32 = 30.0;
const DEATH_HOLD: Seconds = Seconds(0.75);
const AUTHORED_RATE: PlaybackRate = PlaybackRate(1.0);
const PAD: f32 = 12.0;
const TARGET_WIDTH: f32 = 1600.0;
const LABEL_BAND: f32 = 36.0;
const CAPTION_BAND: f32 = 56.0;
const COLUMNS: [(Direction, &str); 8] = [
    (Direction::S, "S"),
    (Direction::SW, "SW"),
    (Direction::W, "W"),
    (Direction::NW, "NW"),
    (Direction::N, "N"),
    (Direction::NE, "NE"),
    (Direction::E, "E"),
    (Direction::SE, "SE"),
];

#[derive(serde::Deserialize)]
struct Config {
    assets_dir: PathBuf,
}

struct Clip {
    model: model::Id,
    action: Action,
    frames_per_loop: usize,
    loops: usize,
}

impl Clip {
    fn frames(&self) -> usize {
        self.frames_per_loop * self.loops
    }
}

#[derive(Resource)]
struct Reel {
    clips: Vec<Clip>,
    sheets: BTreeMap<model::Id, Handle<Image>>,
    target: Handle<Image>,
    next: usize,
    total: usize,
}

#[derive(Resource)]
struct Sink {
    ffmpeg: Child,
    stdin: Option<ChildStdin>,
    pending: BTreeMap<usize, Vec<u8>>,
    written: usize,
}

#[derive(Component)]
struct Slot(Direction);

#[derive(Component)]
struct Caption;

struct Layout {
    zoom: f32,
    cell: Size<WorldPx>,
    feet: f32,
    width: u32,
    height: u32,
}

fn main() {
    let config: Config = envy::prefixed("RIFT_RENDER_")
        .from_env()
        .expect("RIFT_RENDER_* environment");
    let assets_dir = std::fs::canonicalize(&config.assets_dir)
        .unwrap_or_else(|error| fail(&format!("{}: {error}", config.assets_dir.display())));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [names, animations, loops, out] = args.as_slice() else {
        fail("usage: render-actor <names|\"\"> <animations|\"\"> <loops> <out.mp4>");
    };
    let out = PathBuf::from(out);

    let service = AssetService::new(FilesystemSource(assets_dir.clone()));
    let models = select_models(names);
    let loops: usize = loops
        .parse()
        .ok()
        .filter(|&loops| loops > 0)
        .unwrap_or_else(|| {
            fail(&format!(
                "loops must be a positive whole number, not `{loops}`"
            ))
        });
    let clips = plan(&service, &models, animations, loops);
    let layout = layout(&service, &models);
    let total = clips.iter().map(Clip::frames).sum();
    let seconds = total as f32 / FPS;
    eprintln!(
        "render-actor: {} clip(s), {seconds:.1}s of video at {}x{}",
        clips.len(),
        layout.width,
        layout.height
    );

    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                close_when_requested: false,
                ..default()
            })
            .set(AssetPlugin {
                file_path: assets_dir.to_string_lossy().into_owned(),
                ..default()
            })
            .set(ImagePlugin::default_nearest())
            .set(RenderPlugin {
                render_creation: RenderCreation::Automatic(Box::new(WgpuSettings {
                    backends: Some(Backends::VULKAN),
                    ..default()
                })),
                synchronous_pipeline_compilation: true,
                ..default()
            })
            .set(LogPlugin {
                level: Level::WARN,
                filter: "wgpu=error,naga=error,bevy_render=warn".to_owned(),
                ..default()
            }),
    )
    .insert_resource(ClearColor(Color::srgb_u8(0x24, 0x27, 0x2e)))
    .insert_resource(service)
    .insert_resource(Sink::spawn(&out, layout.width, layout.height))
    .add_systems(Update, advance);

    let started = std::time::Instant::now();
    setup(app.world_mut(), clips, total, &layout);
    app.finish();
    app.cleanup();
    let budget = total * 4 + 2000;
    for _ in 0..budget {
        app.update();
        if app.world().resource::<Sink>().written >= total {
            break;
        }
    }
    let mut sink = app.world_mut().remove_resource::<Sink>().expect("sink");
    if sink.written < total {
        fail(&format!("captured only {} of {total} frames", sink.written));
    }
    sink.finish();
    eprintln!(
        "render-actor: rendered in {:.1}s",
        started.elapsed().as_secs_f32()
    );
    println!("{}", out.display());
}

fn select_models(names: &str) -> Vec<model::Id> {
    let wanted: Vec<&str> = split(names);
    if wanted.is_empty() {
        return model::Id::VARIANTS.to_vec();
    }
    wanted
        .into_iter()
        .map(|name| {
            name.replace(['-', '_', ' '], "")
                .parse()
                .unwrap_or_else(|_| {
                    let known: Vec<String> = model::Id::VARIANTS
                        .iter()
                        .map(|id| format!("{id:?}"))
                        .collect();
                    fail(&format!(
                        "no actor named `{name}`; actors: {}",
                        known.join(", ")
                    ))
                })
        })
        .collect()
}

fn plan(service: &AssetService, models: &[model::Id], animations: &str, loops: usize) -> Vec<Clip> {
    let wanted: Vec<Action> = split(animations)
        .into_iter()
        .map(|name| {
            Action::named(name).unwrap_or_else(|| {
                let known: Vec<&str> = Action::ALL.map(Action::name).to_vec();
                fail(&format!(
                    "no animation named `{name}`; animations: {}",
                    known.join(", ")
                ))
            })
        })
        .collect();
    let mut clips = Vec::new();
    for &id in models {
        let model = resolve(service, id);
        for action in Action::ALL {
            if wanted.is_empty() || wanted.contains(&action) {
                clips.push(Clip {
                    model: id,
                    action,
                    frames_per_loop: frames_per_loop(model, action),
                    loops,
                });
            }
        }
    }
    clips
}

fn frames_per_loop(model: &ActorModel, action: Action) -> usize {
    let longest = COLUMNS
        .iter()
        .map(|&(dir, _)| model.timing(action, dir).duration)
        .fold(Seconds(0.0), |a, b| if b > a { b } else { a });
    let length = if action == Action::Dead {
        longest + DEATH_HOLD
    } else {
        longest
    };
    ((length.0 * FPS).round() as usize).max(1)
}

fn layout(service: &AssetService, models: &[model::Id]) -> Layout {
    let drawn = models
        .iter()
        .map(|&id| {
            let model = resolve(service, id);
            model.drawn_size(model.frame(Action::Idle, Direction::S, Seconds(0.0), AUTHORED_RATE))
        })
        .fold(Size::new(0.0, 0.0), |a: Size<WorldPx>, b| {
            Size::new(a.width.max(b.width), a.height.max(b.height))
        });
    let cell = Size::new(drawn.width + PAD, drawn.height + PAD);
    let zoom = (TARGET_WIDTH / (cell.width * COLUMNS.len() as f32))
        .floor()
        .max(1.0);
    let even = |px: f32| (px.ceil() as u32).div_ceil(2) * 2;
    Layout {
        zoom,
        cell,
        feet: PAD / 2.0 + drawn.height * 2.0 / 3.0,
        width: even(cell.width * COLUMNS.len() as f32 * zoom),
        height: even(cell.height * zoom + LABEL_BAND + CAPTION_BAND),
    }
}

fn setup(world: &mut World, clips: Vec<Clip>, total: usize, layout: &Layout) {
    let service = world.resource::<AssetService>().clone();
    let assets = world.resource::<AssetServer>().clone();
    let sheets: BTreeMap<model::Id, Handle<Image>> = clips
        .iter()
        .map(|clip| {
            let sheet = resolve(&service, clip.model).sheet().to_owned();
            (clip.model, assets.load(sheet))
        })
        .collect();

    let mut texture = Image::new_target_texture(
        layout.width,
        layout.height,
        TextureFormat::Rgba8UnormSrgb,
        None,
    );
    texture.sampler = ImageSampler::nearest();
    let target = world.resource_mut::<Assets<Image>>().add(texture);

    let (width, height) = (
        layout.width as f32 / layout.zoom,
        layout.height as f32 / layout.zoom,
    );
    let camera = world
        .spawn((
            Camera2d,
            RenderTarget::Image(target.clone().into()),
            Projection::Orthographic(OrthographicProjection {
                scaling_mode: ScalingMode::Fixed { width, height },
                ..OrthographicProjection::default_2d()
            }),
            Msaa::Off,
        ))
        .id();

    let band_top = height / 2.0 - LABEL_BAND / layout.zoom;
    let left = -width / 2.0;
    for (column, &(dir, label)) in COLUMNS.iter().enumerate() {
        let x = left + layout.cell.width * (column as f32 + 0.5);
        let shade = if column % 2 == 0 { 0x4f } else { 0x48 };
        world.spawn((
            Sprite::from_color(
                Color::srgb_u8(shade, shade + 0x0a, shade - 0x04),
                Vec2::new(layout.cell.width, layout.cell.height),
            ),
            Transform::from_xyz(x, band_top - layout.cell.height / 2.0, 0.0),
        ));
        world.spawn((
            Sprite::default(),
            ACTOR_ANCHOR,
            Transform::from_xyz(x, band_top - layout.feet, 1.0),
            Visibility::Hidden,
            Slot(dir),
        ));
        world.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(layout.cell.width * layout.zoom * column as f32),
                width: Val::Px(layout.cell.width * layout.zoom),
                top: Val::Px(8.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
            UiTargetCamera(camera),
            children![(
                Text::new(label),
                TextFont::from_font_size(18.0),
                TextColor(Color::srgb_u8(0xb8, 0xbe, 0xc8)),
            )],
        ));
    }
    world.spawn((
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(0.0),
            width: Val::Percent(100.0),
            height: Val::Px(CAPTION_BAND),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        UiTargetCamera(camera),
        children![(
            Text::default(),
            TextFont::from_font_size(26.0),
            TextColor(Color::WHITE),
            Caption,
        )],
    ));

    world.insert_resource(Reel {
        clips,
        sheets,
        target,
        next: 0,
        total,
    });
}

fn advance(
    mut commands: Commands,
    mut reel: ResMut<Reel>,
    service: Res<AssetService>,
    assets: Res<AssetServer>,
    mut slots: Query<(&Slot, &mut Sprite, &mut Visibility)>,
    mut caption: Single<&mut Text, With<Caption>>,
) {
    if reel.next >= reel.total {
        return;
    }
    for (id, sheet) in &reel.sheets {
        match assets.load_state(sheet) {
            LoadState::Loaded => {}
            LoadState::Failed(error) => fail(&format!("{id:?} sheet: {error}")),
            _ => return,
        }
    }
    let (clip, frame) = locate(&reel.clips, reel.next);
    let model = resolve(&service, clip.model);
    let elapsed = Seconds((frame % clip.frames_per_loop) as f32 / FPS);
    for (slot, mut sprite, mut visibility) in &mut slots {
        sprite.image = reel.sheets[&clip.model].clone();
        draw_actor_frame(
            &mut sprite,
            model,
            clip.action,
            slot.0,
            elapsed,
            AUTHORED_RATE,
        );
        *visibility = Visibility::Inherited;
    }
    let loop_number = frame / clip.frames_per_loop + 1;
    caption.set_if_neq(Text::new(format!(
        "{:?} - {}   loop {loop_number}/{}",
        clip.model,
        clip.action.name(),
        clip.loops
    )));
    let index = reel.next;
    commands
        .spawn(Screenshot::image(reel.target.clone()))
        .observe(
            move |captured: On<ScreenshotCaptured>, mut sink: ResMut<Sink>| {
                let rgb = captured
                    .image
                    .clone()
                    .try_into_dynamic()
                    .unwrap_or_else(|error| fail(&format!("frame {index}: {error}")))
                    .to_rgb8()
                    .into_raw();
                sink.accept(index, rgb);
            },
        );
    reel.next += 1;
}

fn locate(clips: &[Clip], mut index: usize) -> (&Clip, usize) {
    for clip in clips {
        if index < clip.frames() {
            return (clip, index);
        }
        index -= clip.frames();
    }
    unreachable!("frame index past the last clip")
}

fn resolve(service: &AssetService, id: model::Id) -> &'static ActorModel {
    service.resolve(id.get().sheet, build_model)
}

fn split(list: &str) -> Vec<&str> {
    list.split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .collect()
}

fn fail(message: &str) -> ! {
    eprintln!("render-actor: {message}");
    std::process::exit(2);
}

impl Sink {
    fn spawn(out: &std::path::Path, width: u32, height: u32) -> Sink {
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir)
                .unwrap_or_else(|error| fail(&format!("{}: {error}", dir.display())));
        }
        let mut ffmpeg = Command::new("ffmpeg")
            .args([
                "-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgb24", "-s",
            ])
            .arg(format!("{width}x{height}"))
            .args(["-r", &FPS.to_string(), "-i", "-"])
            .args(["-c:v", "libx264", "-preset", "medium", "-crf", "18"])
            .args(["-pix_fmt", "yuv420p", "-movflags", "+faststart"])
            .arg(out)
            .stdin(Stdio::piped())
            .spawn()
            .unwrap_or_else(|error| fail(&format!("cannot start ffmpeg: {error}")));
        let stdin = ffmpeg.stdin.take();
        Sink {
            ffmpeg,
            stdin,
            pending: BTreeMap::new(),
            written: 0,
        }
    }

    fn accept(&mut self, index: usize, frame: Vec<u8>) {
        self.pending.insert(index, frame);
        while let Some(frame) = self.pending.remove(&self.written) {
            self.stdin
                .as_mut()
                .expect("ffmpeg input open until finish")
                .write_all(&frame)
                .unwrap_or_else(|error| fail(&format!("ffmpeg stopped reading: {error}")));
            self.written += 1;
        }
    }

    fn finish(&mut self) {
        drop(self.stdin.take());
        let status = self
            .ffmpeg
            .wait()
            .unwrap_or_else(|error| fail(&format!("ffmpeg: {error}")));
        if !status.success() {
            fail(&format!("ffmpeg exited with {status}"));
        }
    }
}
