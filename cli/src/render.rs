//! A picture of a game's own area with its models standing in it, drawn off screen by the world's
//! own camera and lights, to compare how the game looks against a reference picture.
//!
//! The shot names its area, the lens the picture is taken through, its eye and target set above the
//! ground beneath them, and each body standing on the ground in it: its model, where it stands, the
//! spot it faces, the items in its hands, and the moment it holds of its stance's idle or of another
//! clip its skeleton plays. A shot may stand at an hour of the day, and film the day turning from
//! it: a picture a frame into the folder `out` names, `0000.png` on. It may hold the area's weather
//! at a spell of its climate, part way through it, with its clouds drifted for a while and lightning
//! striking at a frame; be drawn at any video settings; and arrive: stand the area for one lens,
//! jump to its own, and film every frame as the area stands anew.
//!
//! Every draw of chance on the client's side is seeded by the shot, and time stands still while the
//! area loads and then steps a frame at a time, so the same shot draws the same picture twice.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use bevy::anti_alias::taa::TemporalAntiAliasing;
use bevy::app::ScheduleRunnerPlugin;
use bevy::camera::RenderTarget;
use bevy::diagnostic::FrameCount;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::gpu_readback::{ReadbackComplete, ReadbackOnce};
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use bevy::render::renderer::RenderAdapterInfo;
use bevy::time::TimeUpdateStrategy;
use bevy::window::ExitCondition;
use engine::core::audio::playback::Listener;
use engine::core::content::Content;
use engine::core::ground::Heightfield;
use engine::core::jobs::Handoff;
use engine::core::math::{Pos, Rng};
use engine::core::render::camera::{self, View, Zoom, world_point};
use engine::core::render::video::VideoPreferences;
use engine::core::render::world::WorldScenePlugin;
use engine::core::render::world::body;
use engine::core::render::world::clouds::CloudDrift;
use engine::core::render::world::gpu::draw::TerrainPatch;
use engine::core::render::world::layers::Layered;
use engine::core::render::world::mannequin::mannequin;
use engine::core::render::world::rig_clip;
use engine::core::render::world::sky::Daylight;
use engine::core::render::{CameraRig, Look};
use engine::core::units::{Degrees, Meters, Radians, Seconds};
use engine::core::vfx::rain::{RainField, Shelter};
use engine::core::vfx::render::VfxMaterialsPlugin;
use engine::systems::GameSettings;
use engine::systems::actor::layering::{self, Layering};
use engine::systems::actor::model::{ActorModel, ModelId};
use engine::systems::actor::skeleton::Clip;
use engine::systems::actor::stance::{self, HandIndex};
use engine::systems::actor::{Action, Appearance, ModelDef, Motion, Pose, appearance, render};
use engine::systems::area::scenery::{self, SceneryPlugin};
use engine::systems::area::{self, Area, AreaDef, AreaId, Kept, Pictured, Weather, rain};
use engine::systems::item::{ItemDef, Wear};
use serde::{Deserialize, Serialize};

/// How far around the target the scenery stands, in times the eye's distance from it: as far as a
/// lens looking down at the target takes in, and the haze beyond.
const SCENERY_REACH: f32 = 4.0;

/// How far across the square of ground `--patch` lays is, and how far apart the ground beneath it
/// is sampled for its height.
const PATCH_SIDE: Meters = Meters(24.0);
const PATCH_PROBE: Meters = Meters(1.0);
/// How far over the highest ground beneath it the square lies, so the two never fight for a pixel.
const PATCH_CLEARANCE: Meters = Meters(0.5);

/// Frames the picture settles for once every body stands dressed, while what it shows loads and
/// the frames it is smoothed over fill.
pub const SETTLE: u32 = 180;

/// How long each frame steps time on, so a shot plays out alike from run to run.
const FRAME_STEP: f64 = 1.0 / 60.0;

/// How far two pictures of one shot may differ, in levels of 255, for the shot to count as drawn
/// alike: less than the eye can tell.
const ALIKE_WITHIN: f64 = 0.5;

/// What a render says once it has drawn: where the picture is, and for a shot that arrives, how
/// many frames after the jump the area stood whole.
#[derive(Serialize, Deserialize, Debug)]
pub struct RenderOutcome {
    pub out: PathBuf,
    pub adapter: String,
    pub stood_after_frames: Option<usize>,
}

/// A shot of an area, as its file holds it.
#[derive(Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Shot {
    pub area: String,
    /// A multiple of 64, so each row read back fills whole blocks of 256 bytes and comes unpadded.
    pub width: u32,
    pub height: u32,
    pub view: ViewSpec,
    /// The hour the area stands at, its light as authored when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hour: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub film: Option<Film>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bodies: Vec<Standing>,
    /// The video settings the picture is drawn at: the highest unless given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<VideoChoice>,
    /// The spell of the area's climate the weather holds at, by its name; the first unless given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spell: Option<String>,
    /// How far through the spell the weather stands, 0 to 1: it turns toward the next spell over
    /// the last of it.
    #[serde(default)]
    pub spell_phase: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lightning: Option<LightningAt>,
    /// How long the clouds have drifted on the spell's wind when the picture is taken, in seconds.
    #[serde(default)]
    pub clouds_time: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrive: Option<Arrive>,
    /// What every draw of chance on the client's side is seeded by.
    #[serde(default)]
    pub seed: u64,
}

/// Where the picture is taken from: a lens of its own, with its eye and target and field of view,
/// or the world's own camera placed as a player would have it.
#[derive(Serialize, Deserialize, Clone, Copy)]
pub enum ViewSpec {
    #[serde(rename = "lens")]
    Lens(Lens),
    #[serde(rename = "camera")]
    Camera(CameraShot),
}

/// The world's camera looking at `at` from `yaw`, tilted down by `pitch`, at `zoom`, framed as the
/// game frames it.
#[derive(Serialize, Deserialize, Clone, Copy)]
#[serde(deny_unknown_fields)]
pub struct CameraShot {
    pub at: Spot,
    pub yaw: Degrees,
    pub pitch: Degrees,
    pub zoom: Zoom,
}

/// The day turning `hours` on from the shot's hour over `frames` pictures.
#[derive(Serialize, Deserialize, Clone, Copy)]
#[serde(deny_unknown_fields)]
pub struct Film {
    pub hours: f32,
    pub frames: u32,
}

/// Video settings by a preset's name, or each setting given.
#[derive(Serialize, Deserialize, Clone, Copy)]
#[serde(untagged)]
pub enum VideoChoice {
    Preset(VideoPreset),
    Each(VideoPreferences),
}

/// The video settings a picture is drawn or a bench watches at: every costly part of the picture
/// at its best, and every one off.
#[derive(
    Serialize,
    Deserialize,
    schemars::JsonSchema,
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
)]
#[serde(rename_all = "lowercase")]
pub enum VideoPreset {
    Highest,
    Lowest,
}

impl VideoPreset {
    pub const BOTH: [VideoPreset; 2] = [VideoPreset::Highest, VideoPreset::Lowest];

    pub fn name(self) -> &'static str {
        match self {
            VideoPreset::Highest => "highest",
            VideoPreset::Lowest => "lowest",
        }
    }

    pub fn preferences(self) -> VideoPreferences {
        match self {
            VideoPreset::Highest => VideoPreferences::default(),
            VideoPreset::Lowest => VideoPreferences::lowest(),
        }
    }
}

/// Which of the video presets a command runs at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum VideoFilter {
    Both,
    Highest,
    Lowest,
}

impl VideoFilter {
    pub fn takes(self, preset: VideoPreset) -> bool {
        match self {
            VideoFilter::Both => true,
            VideoFilter::Highest => preset == VideoPreset::Highest,
            VideoFilter::Lowest => preset == VideoPreset::Lowest,
        }
    }

    pub fn presets(self) -> Vec<VideoPreset> {
        VideoPreset::BOTH
            .into_iter()
            .filter(|&preset| self.takes(preset))
            .collect()
    }
}

impl VideoChoice {
    fn preferences(self) -> VideoPreferences {
        match self {
            VideoChoice::Preset(preset) => preset.preferences(),
            VideoChoice::Each(preferences) => preferences,
        }
    }
}

/// Lightning striking close by at the `at_frame`th frame of the settling, lighting that frame as
/// the climate's lightning does; the picture is taken at frame [`SETTLE`].
#[derive(Serialize, Deserialize, Clone, Copy)]
#[serde(deny_unknown_fields)]
pub struct LightningAt {
    pub at_frame: u32,
}

/// Arriving: the area stood for the view `from`, then the camera jumped to the shot's own, and
/// every frame from the jump to the `frames`th taken as the area stands anew.
#[derive(Serialize, Deserialize, Clone, Copy)]
#[serde(deny_unknown_fields)]
pub struct Arrive {
    pub from: ViewSpec,
    pub frames: u32,
}

#[derive(Serialize, Deserialize, Clone, Copy)]
#[serde(deny_unknown_fields)]
pub struct Lens {
    pub eye: Spot,
    pub target: Spot,
    pub fov: Degrees,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Standing {
    pub model: String,
    pub at: Place,
    pub faces: Place,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub held: Vec<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip: Option<Clip>,
    pub moment: Seconds,
}

/// A point `height` above the ground at `x`, `y`, so a lens frames alike on any area's terrain.
#[derive(Serialize, Deserialize, Clone, Copy)]
#[serde(deny_unknown_fields)]
pub struct Spot {
    pub x: Meters,
    pub y: Meters,
    pub height: Meters,
}

/// A view as the renderer places it: where the eye and the target are, and the field of view.
#[derive(Clone, Copy)]
struct Framed {
    eye: Vec3,
    target: Vec3,
    target_ground: Pos<Meters>,
    target_height: Meters,
    fov: Degrees,
}

impl ViewSpec {
    fn framed(self, ground: &Heightfield, look: &Look) -> Framed {
        match self {
            ViewSpec::Lens(lens) => Framed {
                eye: lens.eye.point(ground),
                target: lens.target.point(ground),
                target_ground: lens.target.ground(),
                target_height: lens.target.height,
                fov: lens.fov,
            },
            ViewSpec::Camera(camera) => {
                let mut rig = CameraRig::new(look.camera);
                rig.look_at(
                    camera.at.ground(),
                    Meters(ground.height_at(camera.at.ground()).0 + camera.at.height.0),
                );
                rig.put(camera.yaw, camera.pitch, camera.zoom);
                Framed {
                    eye: rig.eye(),
                    target: rig.target(),
                    target_ground: camera.at.ground(),
                    target_height: Meters(rig.target().y),
                    fov: look.camera.fov,
                }
            }
        }
    }
}

impl Spot {
    fn ground(self) -> Pos<Meters> {
        Pos::new(self.x.0, self.y.0)
    }

    fn point(self, ground: &Heightfield) -> Vec3 {
        let at = self.ground();
        world_point(at, Meters(ground.height_at(at).0 + self.height.0))
    }
}

#[derive(Serialize, Deserialize, Clone, Copy)]
#[serde(deny_unknown_fields)]
pub struct Place {
    pub x: Meters,
    pub y: Meters,
}

impl Place {
    fn ground(self) -> Pos<Meters> {
        Pos::new(self.x.0, self.y.0)
    }
}

#[derive(Resource)]
struct Taking {
    shot: Shot,
    out: PathBuf,
    image: Handle<Image>,
    stage: Stage,
    stood: Option<Stood>,
    frame: u32,
    patch: bool,
}

/// The area as it stands for the picture, for the day to turn over it and its weather to go on.
struct Stood {
    id: AreaId,
    area: Arc<Area>,
    scenery: scenery::Scenery,
    camera: Entity,
    weather: Weather,
}

enum Stage {
    Empty,
    Standing(Vec<Body>),
    Settling(u32),
    Reading(Entity),
    /// Every frame since the jump taken, each read back once drawn, and after how many the area
    /// stood whole.
    Arriving {
        frames: u32,
        readbacks: Vec<Entity>,
        stood_after: Option<u32>,
    },
    /// Every frame taken, waiting for the last to be read back.
    Draining {
        readbacks: Vec<Entity>,
        stood_after: Option<u32>,
    },
}

/// Frames a picture settles for once the hour turns, for the light to reach every pass.
const TURNING: u32 = 2;

struct Body {
    rig: Entity,
    model: ModelId,
    props: Vec<Prop>,
    hung: bool,
}

struct Prop {
    hand: HandIndex,
    joint: String,
    scene: String,
    transform: Transform,
}

#[derive(Resource, Default)]
struct Read(Vec<(Entity, Vec<u8>)>);

pub fn area(folder: &Path, shot: &Path, out: &Path, patch: bool) {
    let folder = std::fs::canonicalize(folder).expect("a game folder");
    let shot: Shot =
        serde_json::from_slice(&std::fs::read(shot).expect("a shot file")).expect("a shot");
    assert!(
        shot.width.is_multiple_of(64),
        "a shot's width is a multiple of 64"
    );
    let content = crate::folder::load(
        &folder,
        "the game did not load",
        engine::core::content::LoadPurpose::Play,
    )
    .unwrap_or_else(|report| panic!("{report}"));
    let out = out.to_path_buf();
    if shot.film.is_some() || shot.arrive.is_some() {
        std::fs::create_dir_all(&out).expect("a folder to film into");
    }
    let video = shot
        .video
        .map_or_else(VideoPreferences::default, VideoChoice::preferences);
    let seed = shot.seed;
    let mut shot = Some(shot);
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
        .add_plugins(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
            FRAME_STEP,
        )))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            FRAME_STEP,
        )))
        .insert_resource(Rng::new(seed))
        .insert_resource(Handoff::NextFrame)
        .add_plugins((WorldScenePlugin, VfxMaterialsPlugin, SceneryPlugin))
        .insert_resource(content.built::<GameSettings>().look)
        .insert_resource(content)
        .insert_resource(video)
        .init_resource::<Read>()
        .init_resource::<RainField>()
        .init_resource::<Shelter>()
        .init_resource::<Listener>()
        .add_observer(|read: On<ReadbackComplete>, mut taken: ResMut<Read>| {
            taken.0.push((read.entity, read.data.clone()));
        })
        .add_systems(Startup, move |world: &mut World| {
            world.resource_mut::<Time<Virtual>>().pause();
            let shot = shot.take().expect("startup runs once");
            let mut image = Image::new_target_texture(
                shot.width,
                shot.height,
                TextureFormat::Rgba8UnormSrgb,
                None,
            );
            image.sampler = ImageSampler::linear();
            image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
            let image = world.resource_mut::<Assets<Image>>().add(image);
            world.insert_resource(Taking {
                shot,
                out: out.clone(),
                image,
                stage: Stage::Empty,
                stood: None,
                frame: 0,
                patch,
            });
        })
        .add_systems(Update, step)
        .run();
}

/// Draws `shot` twice, each in a process of its own, and tells how far the two pictures differ:
/// refused when they differ by more than the eye can tell. Of an arrival, the frames before the
/// area stood whole in both runs, and the frames the picture settles over after that, come as
/// the loading happens to go and are left out.
pub fn check_determinism(folder: &Path, shot: &Path, out: &Path) -> Result<(), String> {
    let again = tempfile::tempdir().map_err(|error| error.to_string())?;
    let twin = again.path().join(out.file_name().ok_or("an output name")?);
    let mut stood = Vec::new();
    for out in [out, twin.as_path()] {
        let rift = std::env::current_exe().map_err(|error| error.to_string())?;
        let drawn = Command::new(rift)
            .args(["render", "area"])
            .args([folder, shot, out])
            .output()
            .map_err(|error| format!("rift doesn't run again: {error}"))?;
        println!("{}", String::from_utf8_lossy(&drawn.stdout).trim_end());
        if !drawn.status.success() {
            return Err(format!(
                "the render of {} failed: {}",
                shot.display(),
                drawn.status
            ));
        }
        let outcome: RenderOutcome = String::from_utf8_lossy(&drawn.stdout)
            .lines()
            .rev()
            .find_map(|line| serde_json::from_str(line).ok())
            .ok_or("the render told no outcome")?;
        stood.extend(outcome.stood_after_frames);
    }
    let from = stood
        .iter()
        .max()
        .map_or(0, |stood| stood + TAA_SETTLE as usize);
    let (rms, pictures) = rms_between(out, &twin, from)?;
    println!("determinism rms={rms:.4}/255 over {pictures} pictures");
    if rms < ALIKE_WITHIN {
        Ok(())
    } else {
        Err(format!(
            "two renders of {} differ by {rms:.4}/255 RMS, more than {ALIKE_WITHIN}",
            shot.display()
        ))
    }
}

/// Frames after which what the picture's history held before a change, blended in a little less
/// each frame, has faded below what the eye can tell.
pub const TAA_SETTLE: u32 = 24;

/// The root mean square difference, in levels of 255, between the picture or folder of pictures
/// at `a` and the same at `b`, the first `from` of a folder's left out, and how many pictures were
/// compared.
fn rms_between(a: &Path, b: &Path, from: usize) -> Result<(f64, usize), String> {
    let pictures: Vec<(PathBuf, PathBuf)> = if a.is_dir() {
        let mut names: Vec<PathBuf> = std::fs::read_dir(a)
            .map_err(|error| error.to_string())?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "png"))
            .collect();
        names.sort();
        names
            .into_iter()
            .skip(from)
            .map(|path| {
                let name = path.file_name().expect("a picture's name").to_owned();
                (path, b.join(name))
            })
            .collect()
    } else {
        vec![(a.to_path_buf(), b.to_path_buf())]
    };
    let mut sum = 0.0f64;
    let mut count = 0usize;
    for (a, b) in &pictures {
        let open = |path: &Path| {
            image::open(path)
                .map(|picture| picture.to_rgb8())
                .map_err(|error| format!("{}: {error}", path.display()))
        };
        let (a, b) = (open(a)?, open(b)?);
        if a.dimensions() != b.dimensions() {
            return Err("the two pictures differ in size".to_owned());
        }
        for (a, b) in a.as_raw().iter().zip(b.as_raw()) {
            let difference = f64::from(*a) - f64::from(*b);
            sum += difference * difference;
        }
        count += a.as_raw().len();
    }
    if count == 0 {
        return Err("no pictures to compare".to_owned());
    }
    Ok(((sum / count as f64).sqrt(), pictures.len()))
}

fn step(world: &mut World) {
    let content = world.resource::<Content>().clone();
    world.resource_scope(|world, mut taking: Mut<Taking>| {
        let taking = &mut *taking;
        match &mut taking.stage {
            Stage::Empty => {
                if let Some((bodies, stood)) =
                    set_up(world, &content, &taking.shot, &taking.image, taking.patch)
                {
                    taking.stage = Stage::Standing(bodies);
                    taking.stood = Some(stood);
                }
            }
            Stage::Standing(bodies) => {
                let mut all = true;
                for standing in bodies.iter_mut() {
                    let model = loaded(&content, standing.model);
                    if !standing.hung {
                        standing.hung = hang(world, model, standing);
                    }
                    all &= standing.hung;
                }
                drive(world, bodies);
                shine(world, taking.stood.as_ref().expect("the area stood"), false);
                if all && !scenery::standing(world) && !engine::core::render::world::readying(world)
                {
                    world.resource_mut::<Time<Virtual>>().unpause();
                    world.resource_mut::<FrameCount>().0 = 0;
                    forget_history(world);
                    taking.stage = match taking.shot.arrive {
                        Some(_) => {
                            jump(world, &taking.shot, taking.stood.as_ref().expect("stood"));
                            Stage::Arriving {
                                frames: 0,
                                readbacks: Vec::new(),
                                stood_after: None,
                            }
                        }
                        None => Stage::Settling(0),
                    };
                }
            }
            Stage::Arriving {
                frames,
                readbacks,
                stood_after,
            } => {
                shine(world, taking.stood.as_ref().expect("the area stood"), false);
                if stood_after.is_none()
                    && !scenery::standing(world)
                    && !engine::core::render::world::readying(world)
                {
                    *stood_after = Some(*frames);
                    forget_history(world);
                }
                readbacks.push(
                    world
                        .spawn(ReadbackOnce::texture(taking.image.clone()))
                        .id(),
                );
                *frames += 1;
                let wanted = taking.shot.arrive.expect("an arriving shot").frames;
                if *frames >= wanted {
                    taking.stage = Stage::Draining {
                        readbacks: std::mem::take(readbacks),
                        stood_after: *stood_after,
                    };
                }
            }
            Stage::Draining {
                readbacks,
                stood_after,
            } => {
                let read = world.resource::<Read>();
                if !readbacks
                    .iter()
                    .all(|readback| read.0.iter().any(|(entity, _)| entity == readback))
                {
                    return;
                }
                let mut read = world.resource_mut::<Read>();
                for (frame, readback) in readbacks.iter().enumerate() {
                    let index = read
                        .0
                        .iter()
                        .position(|(entity, _)| entity == readback)
                        .expect("every frame read back");
                    let pixels = read.0.remove(index).1;
                    let path = taking.out.join(format!("{frame:04}.png"));
                    save(pixels, taking.shot.width, taking.shot.height, &path);
                }
                done(world, &taking.out, *stood_after);
            }
            Stage::Settling(frames) => {
                *frames += 1;
                let flashing = taking
                    .shot
                    .lightning
                    .is_some_and(|lightning| lightning.at_frame == *frames);
                shine(
                    world,
                    taking.stood.as_ref().expect("the area stood"),
                    flashing,
                );
                if *frames >= SETTLE {
                    let readback = world
                        .spawn(ReadbackOnce::texture(taking.image.clone()))
                        .id();
                    taking.stage = Stage::Reading(readback);
                }
            }
            Stage::Reading(readback) => {
                let mut read = world.resource_mut::<Read>();
                let Some(index) = read.0.iter().position(|(entity, _)| entity == readback) else {
                    return;
                };
                let pixels = read.0.remove(index).1;
                let Some(film) = taking.shot.film else {
                    save(pixels, taking.shot.width, taking.shot.height, &taking.out);
                    done(world, &taking.out, None);
                    return;
                };
                let path = taking.out.join(format!("{:04}.png", taking.frame));
                save(pixels, taking.shot.width, taking.shot.height, &path);
                taking.frame += 1;
                if taking.frame >= film.frames {
                    done(world, &taking.out, None);
                    return;
                }
                let start = taking.shot.hour.expect("a filmed shot stands at an hour");
                let hour = start + film.hours * taking.frame as f32 / film.frames as f32;
                world.insert_resource(Daylight::Hour(hour.rem_euclid(24.0)));
                taking.stage = Stage::Settling(SETTLE - TURNING);
            }
        }
    });
}

/// Starts the picture's temporal smoothing over from the frame being drawn, so that what the
/// frames before it showed, however many the area took to load, leaves no trace in the picture.
fn forget_history(world: &mut World) {
    for mut smoothing in world.query::<&mut TemporalAntiAliasing>().iter_mut(world) {
        smoothing.reset = true;
    }
}

fn done(world: &mut World, out: &Path, stood_after_frames: Option<u32>) {
    let outcome = RenderOutcome {
        out: out.to_path_buf(),
        adapter: world
            .get_resource::<RenderAdapterInfo>()
            .map(|adapter| adapter.name.clone())
            .unwrap_or_default(),
        stood_after_frames: stood_after_frames.map(|frames| frames as usize),
    };
    println!(
        "{}",
        serde_json::to_string(&outcome).expect("a serializable outcome")
    );
    world.write_message(AppExit::Success);
}

/// Stands the shot's area and its bodies and points the camera, once the area's cover shaders are
/// compiled.
fn set_up(
    world: &mut World,
    content: &Content,
    shot: &Shot,
    image: &Handle<Image>,
    patch: bool,
) -> Option<(Vec<Body>, Stood)> {
    let area_id = content
        .by_path::<AreaDef>(&shot.area)
        .unwrap_or_else(|| panic!("no area {}", shot.area));
    let picture = area::picture(content, Pictured::Area(area_id), Kept::All)
        .unwrap_or_else(|| panic!("{} does not picture", shot.area));
    if !scenery::shaders_ready(world, content, &picture.area) {
        return None;
    }
    let look = *world.resource::<Look>();
    let view = shot.arrive.map_or(shot.view, |arrive| arrive.from);
    let framed = view.framed(&picture.area.heights, &look);
    let placed = Transform::from_translation(framed.eye).looking_at(framed.target, Vec3::Y);
    let camera = world
        .spawn((
            Camera3d::default(),
            Camera {
                clear_color: ClearColorConfig::Custom(Color::BLACK),
                ..default()
            },
            RenderTarget::Image(image.clone().into()),
            camera::lens(framed.fov, shot.width, shot.height),
            camera::picture(look.picture),
            placed,
        ))
        .id();
    let mut rig = CameraRig::new(look.camera);
    rig.look_at(framed.target_ground, framed.target_height);
    world.insert_resource(rig);
    world.insert_resource(CloudDrift::carried_for(shot.clouds_time));
    let stood = scenery::spawn(world, content, &picture.area, default());
    see(world, stood, framed, shot);
    if patch {
        let target = shot.view.framed(&picture.area.heights, &look).target_ground;
        world.spawn(patch_at(&picture.area.heights, target));
    }
    world.insert_resource(shot.hour.map_or(Daylight::Fixed, Daylight::Hour));
    let climate = &picture.area.climate;
    let spell = match &shot.spell {
        Some(name) => climate
            .spell_named(name)
            .unwrap_or_else(|| panic!("{} has no spell named {name}", shot.area)),
        None => 0,
    };
    // Lightning strikes only where the shot says, never by chance.
    let weather = Weather {
        lightning: 0.0,
        ..climate.held_at(spell, shot.spell_phase)
    };
    let stood = Stood {
        id: area_id,
        area: picture.area.clone(),
        scenery: stood,
        camera,
        weather,
    };
    shine(world, &stood, false);
    let bodies = shot
        .bodies
        .iter()
        .map(|standing| stand(world, content, &picture.area.heights, standing))
        .collect();
    Some((bodies, stood))
}

/// A square of ground drawn by the GPU draw path, centred on `at` and lying a little over the
/// highest ground beneath it, clear of the world's own.
fn patch_at(ground: &Heightfield, at: Pos<Meters>) -> TerrainPatch {
    let corner = Pos::new(at.x - PATCH_SIDE.0 / 2.0, at.y - PATCH_SIDE.0 / 2.0);
    let steps = (PATCH_SIDE.0 / PATCH_PROBE.0).ceil() as u32;
    let highest = (0..=steps)
        .flat_map(|row| (0..=steps).map(move |column| (row, column)))
        .map(|(row, column)| {
            let across = PATCH_SIDE.0 * column as f32 / steps as f32;
            let down = PATCH_SIDE.0 * row as f32 / steps as f32;
            ground
                .height_at(Pos::new(corner.x + across, corner.y + down))
                .0
        })
        .fold(f32::MIN, f32::max);
    TerrainPatch::new(corner, PATCH_SIDE, Meters(highest + PATCH_CLEARANCE.0))
}

/// Tells the scenery where the view stands and how far around it to stand.
fn see(world: &mut World, stood: scenery::Scenery, framed: Framed, shot: &Shot) {
    scenery::view_from(
        world,
        stood,
        framed.target_ground,
        Meters(framed.eye.distance(framed.target) * SCENERY_REACH),
        View::new(
            Transform::from_translation(framed.eye).looking_at(framed.target, Vec3::Y),
            Vec2::new(shot.width as f32, shot.height as f32),
            framed.fov,
        ),
    );
}

/// Jumps the camera from the view the area stood for to the shot's own.
fn jump(world: &mut World, shot: &Shot, stood: &Stood) {
    let look = *world.resource::<Look>();
    let framed = shot.view.framed(&stood.area.heights, &look);
    world.entity_mut(stood.camera).insert((
        camera::lens(framed.fov, shot.width, shot.height),
        Transform::from_translation(framed.eye).looking_at(framed.target, Vec3::Y),
    ));
    world
        .resource_mut::<CameraRig>()
        .look_at(framed.target_ground, framed.target_height);
    see(world, stood.scenery, framed, shot);
}

/// Lights the stood area under its weather at the moment and lets its rain fall, as the client
/// does every frame; lightning striking close by this frame when `flashing`.
fn shine(world: &mut World, stood: &Stood, flashing: bool) {
    let daylight = *world.resource::<Daylight>();
    scenery::light(
        world,
        &stood.area,
        &stood.weather,
        daylight,
        stood.scenery,
        stood.camera,
    );
    rain::fall(
        world,
        stood.id,
        &stood.area,
        &stood.weather,
        stood.scenery.rain,
        stood.camera,
    );
    let flash = if flashing { 1.0 } else { 0.0 };
    if flashing && let Some(lightning) = stood.area.climate.lightning {
        let day = world.resource::<Look>().light.ambient_nits;
        if let Some(mut ambient) = world.get_mut::<AmbientLight>(stood.camera) {
            ambient.brightness += lightning.flash * day;
        }
    }
    world.insert_resource(scenery::shown_weather(&stood.weather, flash));
}

fn loaded(content: &Content, model: ModelId) -> &ActorModel {
    ActorModel::loaded(content, model)
        .unwrap_or_else(|| panic!("{} did not build", model.path(content)))
}

/// Stands a body where the shot says, facing its spot, holding its items in its stance's idle.
fn stand(world: &mut World, content: &Content, ground: &Heightfield, standing: &Standing) -> Body {
    let model_id = content
        .by_path::<ModelDef>(&standing.model)
        .unwrap_or_else(|| panic!("no model {}", standing.model));
    let model = loaded(content, model_id);
    let held: BTreeMap<HandIndex, _> = standing
        .held
        .iter()
        .enumerate()
        .filter_map(|(hand, item)| {
            let item = item.as_ref()?;
            let id = content
                .by_path::<ItemDef>(item)
                .unwrap_or_else(|| panic!("no item {item}"));
            Some((HandIndex(hand as u8), id))
        })
        .collect();
    let stance = stance::of_hands(content, &held);
    let built = model.skeleton();
    let played = match &standing.clip {
        Some(clip) => built
            .played(clip)
            .unwrap_or_else(|| panic!("{} plays no clip {clip}", standing.model)),
        None => model.standing_clip(stance, &Motion::Action(Action::Idle), Pose::Idle),
    };
    let clip = &built.clips[played];
    let pairs = clip
        .source
        .map(|source| built.def.sources[source].joint_pairs());
    let made = rig_clip::rig_clip(
        world,
        built.rig.clone(),
        clip.clip.file().as_str(),
        clip.animation,
        rig_clip::maker(built.rig.clone(), pairs),
    );
    let mut mannequin = mannequin(world, made);
    mannequin.hold(Some(standing.moment));
    let figure = model.dressed(
        content,
        &appearance::look(content, &Appearance::of(model_id), &BTreeMap::new()),
    );
    let at = standing.at.ground();
    let faces = standing.faces.ground();
    let rig = world
        .spawn((
            Transform::from_translation(world_point(at, ground.height_at(at))).with_rotation(
                camera::facing_rotation(Radians((faces.y - at.y).atan2(faces.x - at.x))),
            ),
            Visibility::default(),
            mannequin,
        ))
        .id();
    body::dress(world, rig, &figure);
    let props = render::hung(content, model, &held)
        .into_iter()
        .filter_map(|(hand, item)| {
            let Some(Wear::Held { grip, .. }) = item.get(content).wear() else {
                return None;
            };
            let at = model.hands().get(hand.index())?;
            Some(Prop {
                hand,
                joint: at.joint.0.clone(),
                scene: item.get(content).look.get(content).model.path(),
                transform: render::held_at(model, at, grip),
            })
        })
        .collect();
    Body {
        rig,
        model: model_id,
        props,
        hung: false,
    }
}

/// Hangs the body's props under their joints and puts its layers on, once its scene has spawned
/// the joints and every part of it shows.
fn hang(world: &mut World, model: &ActorModel, standing: &Body) -> bool {
    let Some(joints) = world
        .get::<body::Body>(standing.rig)
        .map(|body| body.joints().clone())
    else {
        return false;
    };
    let rigged = model
        .skeleton()
        .rig
        .joints()
        .all(|joint| joints.contains_key(&model.skeleton().rig.nodes[joint].name));
    let dressed = world
        .get::<Children>(standing.rig)
        .into_iter()
        .flatten()
        .filter(|child| world.get::<WorldAssetRoot>(**child).is_some())
        .all(|skin| world.get::<Visibility>(*skin) != Some(&Visibility::Hidden));
    if !rigged || !dressed {
        return false;
    }
    for prop in &standing.props {
        let Some(&joint) = joints.get(&prop.joint) else {
            continue;
        };
        let scene = world
            .resource::<AssetServer>()
            .load(GltfAssetLabel::Scene(0).from_asset(prop.scene.clone()));
        world.spawn((
            WorldAssetRoot(scene),
            prop.transform,
            Visibility::default(),
            ChildOf(joint),
        ));
    }
    if let Some((layered, layering)) = layering::layered(model, &joints) {
        world.entity_mut(standing.rig).insert((layered, layering));
    }
    true
}

/// Drives each body's layers as the game drives an idle body holding its props.
fn drive(world: &mut World, bodies: &[Body]) {
    for standing in bodies {
        let holding: Vec<HandIndex> = standing.props.iter().map(|prop| prop.hand).collect();
        if let Ok((mut layered, layering)) = world
            .query::<(&mut Layered, &Layering)>()
            .get_mut(world, standing.rig)
        {
            layering::drive(&mut layered, layering, Action::Idle, &holding, false);
        }
    }
}

fn save(pixels: Vec<u8>, width: u32, height: u32, path: &std::path::Path) {
    let picture = Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels,
        TextureFormat::Rgba8UnormSrgb,
        default(),
    );
    picture
        .try_into_dynamic()
        .expect("an rgba picture")
        .to_rgba8()
        .save(path)
        .expect("a written picture");
}
