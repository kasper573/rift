//! A sheet of vfx coats, drawn off screen by the engine's own renderer into a video: each tile one
//! vfx played on what it wraps (a body, what the body holds, a prop or a patch of ground), standing
//! alone under the game's daylight in front of a camera of its own, seen turning round once a page.
//! The tiles fill a page in rows, and the pages follow each other, as many as the tiles take.
//!
//! What a tile shows stands still, as everything in a world does, and its camera and the sun go
//! round it together: on screen it turns in a light that stays where it is, while the world sees
//! nothing move, so nothing it does as a thing moves (feet stepping, trails left behind) shows.

use std::collections::{BTreeMap, VecDeque};
use std::f32::consts::TAU;
use std::io::Write;
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::camera::RenderTarget;
use bevy::camera::primitives::Aabb;
use bevy::prelude::*;
use bevy::render::gpu_readback::{ReadbackComplete, ReadbackOnce};
use bevy::render::render_resource::{TextureFormat, TextureUsages};
use bevy::time::TimeUpdateStrategy;
use bevy::window::ExitCondition;
use bevy_egui::{EguiPrimaryContextPass, EguiTextureHandle, EguiUserTextures, PrimaryEguiContext};
use engine::core::audio::playback::Listener;
use engine::core::content::media::GltfInfos;
use engine::core::content::{Content, LoadPurpose};
use engine::core::geometry::{PlacedShape, Shape};
use engine::core::ground::{GroundSize, HeightRange, Heightfield, WaterSurface};
use engine::core::gui::{self, Band, GuiPlugin};
use engine::core::jobs::Handoff;
use engine::core::math::{Pos, Rng};
use engine::core::os::Profile;
use engine::core::render::camera::{self, GroundHeights, ground_height, world_point};
use engine::core::render::sample::FrameSamplingPlugin;
use engine::core::render::video::VideoPreferences;
use engine::core::render::world::shine::drawn_with_in;
use engine::core::render::world::{WorldScenePlugin, body as world_body, lights};
use engine::core::render::{CameraFocus, Look};
use engine::core::time::{ServerClock, UnixMillis, server_now};
use engine::core::units::{Degrees, Meters, Radians, Seconds};
use engine::core::vfx::render::{SiteExtent, VfxHand, VfxRenderPlugin};
use engine::core::vfx::standing::StandingVfx;
use engine::core::vfx::{ParamValue, VfxDef, VfxId, VfxInstance, VfxPart, VfxPlay, VfxSite};
use engine::systems::GameSettings;
use engine::systems::area::rain::{RainField, Shelter};
use engine::systems::area::scenery::{self, SceneryPlugin, ShowsProp};
use engine::systems::area::{PropDef, PropId};
use serde::Deserialize;
use ui::egui::{self, Align2, Color32, FontId, LayerId, Pos2, Rect};

use super::body::{self, Body, Posed};

/// A sheet of vfx coats, as its file holds it: how many tiles each page lays out, how big each is
/// drawn, and every tile, in the order the pages show them.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoatSheet {
    /// How many tiles a page lays side by side.
    pub columns: u32,
    /// How many rows of them a page holds.
    pub rows: u32,
    pub tile: TileSize,
    pub tiles: Vec<CoatTile>,
}

/// How big a tile is drawn, in pixels, its label's bar included. A page of the tiles is a
/// multiple of 64 pixels wide and an even number of pixels high, as it is read back and filmed.
#[derive(Deserialize, Clone, Copy)]
#[serde(deny_unknown_fields)]
pub struct TileSize {
    pub width: u32,
    pub height: u32,
}

/// One vfx on what it wraps, under the label its tile reads.
#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct CoatTile {
    /// What the tile's bar reads, and what a name given to pick tiles by matches.
    pub label: String,
    /// The vfx, by its path in the game folder.
    pub vfx: String,
    /// Its parameters as its carrier fills them, keyed by those the vfx declares.
    #[serde(default)]
    pub fill: BTreeMap<String, ParamValue>,
    pub on: CoatCarrier,
}

/// What a tile's vfx plays on.
#[derive(Deserialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CoatCarrier {
    /// A body of `model`, by its path, holding the items `held` in its hands, hand by hand, as a
    /// shot's body holds them, and standing in its stance's idle; the vfx on `part` of it.
    Body {
        model: String,
        #[serde(default)]
        held: Vec<Option<String>>,
        #[serde(default)]
        part: VfxPart,
    },
    /// The model of a prop, by its path, standing alone.
    Prop { prop: String },
    /// A patch of ground of `shape`, laid about its point.
    Ground { shape: Shape },
}

/// Films the tiles of `sheet` of the game in `folder` whose labels match any of the patterns of
/// `names`, every tile when none is given, as the video `out`, through `ffmpeg` on the path.
pub fn run(folder: &Path, sheet: &Path, out: &Path, names: &[String]) {
    let folder = std::fs::canonicalize(folder).expect("a game folder");
    let sheet: CoatSheet = serde_json::from_slice(
        &std::fs::read(sheet).unwrap_or_else(|error| panic!("{}: {error}", sheet.display())),
    )
    .unwrap_or_else(|error| panic!("{} is no coat sheet: {error}", sheet.display()));
    let page = Page::of(&sheet);
    let patterns: Vec<glob::Pattern> = names
        .iter()
        .map(|name| {
            glob::Pattern::new(name).unwrap_or_else(|error| panic!("{name} is no pattern: {error}"))
        })
        .collect();
    let tiles: Vec<CoatTile> = sheet
        .tiles
        .iter()
        .filter(|tile| {
            patterns.is_empty() || patterns.iter().any(|pattern| pattern.matches(&tile.label))
        })
        .cloned()
        .collect();
    assert!(!tiles.is_empty(), "no tile's label matches {names:?}");
    let content = crate::folder::load(&folder, "the game did not load", LoadPurpose::Play)
        .unwrap_or_else(|report| panic!("{report}"));
    let tiles: Vec<Staged> = tiles
        .into_iter()
        .map(|tile| Staged::of(&content, tile))
        .collect();
    let pages = tiles.chunks(page.slots()).map(<[Staged]>::to_vec).collect();
    let film = Film::start(out, page.width(), page.height());
    let profile = tempfile::tempdir().expect("a scratch profile");
    let mut clock = ServerClock::default();
    clock.record(CLOCK_START, Seconds(0.0));
    let mut app = App::new();
    engine::core::assets::serve_folder(&mut app, &folder);
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            })
            .disable::<bevy::winit::WinitPlugin>(),
    )
    .add_plugins(ScheduleRunnerPlugin::run_loop(Duration::ZERO))
    .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
        1.0 / f64::from(FPS),
    )))
    .insert_resource(Rng::new(0))
    .insert_resource(Handoff::NextFrame)
    .insert_resource(Profile(profile.path().to_path_buf()))
    .add_plugins((
        WorldScenePlugin,
        VfxRenderPlugin,
        SceneryPlugin,
        FrameSamplingPlugin,
        GuiPlugin,
    ))
    .insert_resource(content.built::<GameSettings>().look)
    .insert_resource(content)
    .insert_resource(VideoPreferences::default())
    .insert_resource(clock)
    .insert_resource(page.ground())
    .init_resource::<Read>()
    .init_resource::<RainField>()
    .init_resource::<Shelter>()
    .init_resource::<Listener>()
    .add_observer(|read: On<ReadbackComplete>, mut taken: ResMut<Read>| {
        taken.0.push((read.entity, read.data.clone()));
    })
    .insert_resource(Filming {
        page,
        pages,
        at: 0,
        stage: Stage::Standing(0),
        slots: Vec::new(),
        sun: Entity::PLACEHOLDER,
        picture: Handle::default(),
        film,
    })
    .add_systems(Startup, set_stage)
    .add_systems(Update, step)
    .add_systems(EguiPrimaryContextPass, draw.in_set(Band::Stage))
    .run();
}

/// How long each tile is seen taking to turn round once: how long each page shows.
const REVOLUTION: Seconds = Seconds(4.0);
const FPS: u32 = 30;
/// How long a page plays before it is filmed, for what its vfx give off to fill the air round
/// it as it does while it plays on.
const WARM_UP: Seconds = Seconds(2.0);
/// How many frames in a row nothing may be on its way to standing for before a page plays: a vfx
/// compiles what it draws with only once it is first drawn.
const STEADY: u32 = 10;
/// How far above level each camera looks down at its tile.
const ELEVATION: Degrees = Degrees(20.0);
const FOV: Degrees = Degrees(30.0);
/// How much room a tile's camera leaves round what it frames.
const MARGIN: f32 = 1.1;
/// How far apart the tiles stand, in metres: past what any camera draws, so none shows in
/// another's tile.
const SPACING: f32 = 200.0;
/// How far a camera draws past what it frames, in times its distance from it.
const DEPTH: f32 = 2.0;
/// Where the stage's sun shines from as a page starts: over the right shoulder of every camera.
const SUN_ELEVATION: Degrees = Degrees(40.0);
const SUN_AZIMUTH: Degrees = Degrees(150.0);
/// The share of a tile's height its label's bar takes.
const BAR: f32 = 0.075;
const BACKDROP: Color = Color::srgb(0.16, 0.16, 0.17);
const BAR_FILL: Color32 = Color32::from_rgb(24, 24, 26);
const LABEL_INK: Color32 = Color32::from_rgb(236, 236, 236);
/// The way every tile faces: toward its camera as a page starts, along +Z.
const FRONT: Radians = Radians(std::f32::consts::FRAC_PI_2);
/// The wall clock the stage's vfx start by.
const CLOCK_START: UnixMillis = UnixMillis(1_000_000);
/// What a tile's camera frames before what it shows has stood and can be measured: a body's
/// middle, this high, this far round.
const STANDING_IN: (f32, f32) = (1.0, 1.5);

/// How the tiles lie on a page.
#[derive(Clone, Copy)]
struct Page {
    columns: u32,
    rows: u32,
    tile: TileSize,
}

impl Page {
    fn of(sheet: &CoatSheet) -> Page {
        let page = Page {
            columns: sheet.columns.max(1),
            rows: sheet.rows.max(1),
            tile: sheet.tile,
        };
        assert!(
            page.width().is_multiple_of(64),
            "a page of the sheet is a multiple of 64 pixels wide, not {}",
            page.width()
        );
        assert!(
            page.height().is_multiple_of(2),
            "a page of the sheet is an even number of pixels high, not {}",
            page.height()
        );
        page
    }

    fn slots(self) -> usize {
        (self.columns * self.rows) as usize
    }

    fn width(self) -> u32 {
        self.columns * self.tile.width
    }

    fn height(self) -> u32 {
        self.rows * self.tile.height
    }

    fn bar(self) -> u32 {
        (self.tile.height as f32 * BAR).round() as u32
    }

    /// How tall the picture of a tile's camera is, over its bar.
    fn view_height(self) -> u32 {
        self.tile.height - self.bar()
    }

    /// The flat ground every tile stands on, wide enough for a page of them.
    fn ground(self) -> GroundHeights {
        let size = GroundSize {
            width: (self.slots() as f32 * SPACING).ceil() as u32,
            height: SPACING as u32,
        };
        let range = HeightRange {
            lowest: Meters(-1.0),
            highest: Meters(1.0),
        };
        GroundHeights {
            heights: Heightfield::flat(size, range),
            water: WaterSurface::dry(size, range),
            bounds: size.bounds(),
        }
    }

    /// Where the tile of `slot` stands on the ground.
    fn spot(slot: usize) -> Pos<Meters> {
        Pos::new(SPACING * (slot as f32 + 0.5), SPACING / 2.0)
    }
}

/// A tile with what it names found in the game.
#[derive(Clone)]
struct Staged {
    label: String,
    play: VfxPlay,
    on: CoatCarrier,
}

impl Staged {
    fn of(content: &Content, tile: CoatTile) -> Staged {
        let vfx: VfxId = content
            .by_path::<VfxDef>(&tile.vfx)
            .unwrap_or_else(|| panic!("{}: no vfx {}", tile.label, tile.vfx));
        let part = match &tile.on {
            CoatCarrier::Body { part, .. } => part.clone(),
            CoatCarrier::Prop { .. } | CoatCarrier::Ground { .. } => VfxPart::Whole,
        };
        if let CoatCarrier::Prop { prop } = &tile.on {
            content
                .by_path::<PropDef>(prop)
                .unwrap_or_else(|| panic!("{}: no prop {prop}", tile.label));
        }
        Staged {
            label: tile.label,
            play: VfxPlay {
                vfx,
                part,
                fill: tile.fill,
            },
            on: tile.on,
        }
    }
}

#[derive(Resource)]
struct Filming {
    page: Page,
    pages: Vec<Vec<Staged>>,
    /// Which page is on show.
    at: usize,
    stage: Stage,
    slots: Vec<Slot>,
    sun: Entity,
    /// The page the tiles are laid out on, as it is filmed.
    picture: Handle<Image>,
    film: Film,
}

enum Stage {
    /// The page's tiles stand, and have stood whole for so many frames in a row.
    Standing(u32),
    /// The page's vfx play on for so many frames before it is filmed.
    WarmingUp(u32),
    /// So many frames of the page are filmed.
    Filming(u32),
    /// Every page is filmed, and what is still being read back waits to be.
    Draining,
}

/// A place on the page: the camera that draws its tile, the picture it draws into as the page
/// shows it, and the tile standing there now.
struct Slot {
    camera: Entity,
    texture: egui::TextureId,
    shown: Option<Shown>,
}

/// A tile as it stands: the thing its vfx plays on, where it stands before it turns, the body
/// that thing is, if it is one, and how its camera frames it once it has been measured.
struct Shown {
    label: String,
    thing: Entity,
    body: Option<Body>,
    framing: Framing,
    measured: bool,
}

/// How a camera frames what it shows: the point it looks at, and from how far in front of it.
#[derive(Clone, Copy)]
struct Framing {
    center: Vec3,
    distance: f32,
}

impl Framing {
    /// Where the camera stands, looking down at what it frames from in front of it, gone `round`
    /// it.
    fn eye(self, round: Quat) -> Transform {
        let (rise, level) = ELEVATION.radians().0.sin_cos();
        let way = round * Vec3::new(0.0, rise, level);
        Transform::from_translation(self.center + way * self.distance)
            .looking_at(self.center, Vec3::Y)
    }

    /// Frames a sphere of `radius` about `center` whole.
    fn round(center: Vec3, radius: f32) -> Framing {
        Framing {
            center,
            distance: radius * MARGIN / (FOV.radians().0 / 2.0).sin(),
        }
    }

    fn lens(self, width: u32, height: u32) -> Projection {
        Projection::Perspective(PerspectiveProjection {
            fov: FOV.radians().0,
            aspect_ratio: width as f32 / height.max(1) as f32,
            near: (self.distance / 100.0).max(0.01),
            far: self.distance * DEPTH,
            ..default()
        })
    }
}

/// The video the pages are filmed into: `ffmpeg` taking every frame read back, in order.
struct Film {
    ffmpeg: Child,
    input: Option<ChildStdin>,
    pending: VecDeque<Entity>,
}

impl Film {
    fn start(out: &Path, width: u32, height: u32) -> Film {
        let mut ffmpeg = Command::new("ffmpeg")
            .args([
                "-y",
                "-loglevel",
                "error",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgba",
            ])
            .args(["-s", &format!("{width}x{height}")])
            .args(["-r", &FPS.to_string(), "-i", "-"])
            .args(["-c:v", "libx264", "-crf", "18", "-pix_fmt", "yuv420p"])
            .arg(out)
            .stdin(Stdio::piped())
            .spawn()
            .unwrap_or_else(|error| panic!("ffmpeg doesn't start: {error}"));
        let input = ffmpeg.stdin.take();
        Film {
            ffmpeg,
            input,
            pending: VecDeque::new(),
        }
    }

    /// Hands ffmpeg every frame read back so far whose frames before it are handed already.
    fn take(&mut self, read: &mut Read) {
        while let Some(&next) = self.pending.front() {
            let Some(index) = read.0.iter().position(|(entity, _)| *entity == next) else {
                return;
            };
            let pixels = read.0.remove(index).1;
            self.input
                .as_mut()
                .expect("ffmpeg taking frames")
                .write_all(&pixels)
                .expect("ffmpeg takes the frame");
            self.pending.pop_front();
        }
    }

    fn finish(&mut self) {
        drop(self.input.take());
        let status = self.ffmpeg.wait().expect("ffmpeg ends");
        assert!(status.success(), "ffmpeg failed: {status}");
    }
}

#[derive(Resource, Default)]
struct Read(Vec<(Entity, Vec<u8>)>);

/// Hangs the sun over the stage, and gives each place on the page its camera and the page its
/// own, which lays the tiles out and labels them.
fn set_stage(world: &mut World) {
    let look = *world.resource::<Look>();
    let sun = world
        .spawn(lights::sun(
            Color::WHITE,
            look.light.sun_lux,
            SUN_ELEVATION,
            SUN_AZIMUTH,
        ))
        .id();
    let page = world.resource::<Filming>().page;
    let (width, height) = (page.tile.width, page.view_height());
    let slots: Vec<Slot> = (0..page.slots())
        .map(|slot| {
            let image = target(world, width, height);
            let texture = world
                .resource_mut::<EguiUserTextures>()
                .add_image(EguiTextureHandle::Strong(image.clone()));
            let camera = world
                .spawn((
                    Camera3d::default(),
                    Camera {
                        order: slot as isize,
                        clear_color: ClearColorConfig::Custom(BACKDROP),
                        ..default()
                    },
                    RenderTarget::Image(image.into()),
                    camera::picture(look.picture),
                    AmbientLight {
                        brightness: look.light.ambient_nits,
                        ..default()
                    },
                ))
                .id();
            Slot {
                camera,
                texture,
                shown: None,
            }
        })
        .collect();
    let picture = target(world, page.width(), page.height());
    world.spawn((
        Camera2d,
        Camera {
            order: slots.len() as isize,
            clear_color: ClearColorConfig::Custom(BACKDROP),
            ..default()
        },
        RenderTarget::Image(picture.clone().into()),
        Msaa::Off,
        PrimaryEguiContext,
    ));
    let mut filming = world.resource_mut::<Filming>();
    filming.slots = slots;
    filming.sun = sun;
    filming.picture = picture;
}

fn target(world: &mut World, width: u32, height: u32) -> Handle<Image> {
    let mut image = Image::new_target_texture(width, height, TextureFormat::Rgba8UnormSrgb, None);
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    world.resource_mut::<Assets<Image>>().add(image)
}

fn step(world: &mut World) {
    let content = world.resource::<Content>().clone();
    world.resource_scope(|world, mut filming: Mut<Filming>| {
        let filming = &mut *filming;
        world.resource_scope(|_, mut read: Mut<Read>| filming.film.take(&mut read));
        match filming.stage {
            Stage::Standing(steady) => {
                if filming.slots.iter().all(|slot| slot.shown.is_none()) {
                    for (slot, tile) in filming.pages[filming.at].iter().enumerate() {
                        let shown = stand(world, &content, slot, tile);
                        filming.slots[slot].shown = Some(shown);
                    }
                }
                let mut whole = true;
                for shown in filming
                    .slots
                    .iter_mut()
                    .filter_map(|slot| slot.shown.as_mut())
                {
                    whole &= shown.stood(world, &content);
                }
                drive(world, &filming.slots);
                whole &= !engine::core::render::world::readying(world);
                let steady = if whole { steady + 1 } else { 0 };
                filming.stage = if steady < STEADY {
                    Stage::Standing(steady)
                } else if filming
                    .slots
                    .iter()
                    .any(|slot| slot.shown.as_ref().is_some_and(|shown| !shown.measured))
                {
                    for shown in filming
                        .slots
                        .iter_mut()
                        .filter_map(|slot| slot.shown.as_mut())
                    {
                        shown.measure(world);
                    }
                    Stage::Standing(0)
                } else {
                    camera::forget_history(world);
                    Stage::WarmingUp(0)
                };
                aim(world, filming, 0.0);
            }
            Stage::WarmingUp(frames) => {
                drive(world, &filming.slots);
                aim(world, filming, 0.0);
                filming.stage = if frames + 1 >= frames_of(WARM_UP) {
                    Stage::Filming(0)
                } else {
                    Stage::WarmingUp(frames + 1)
                };
            }
            Stage::Filming(frame) if frame >= frames_of(REVOLUTION) => {
                for slot in &mut filming.slots {
                    if let Some(shown) = slot.shown.take() {
                        world.despawn(shown.thing);
                    }
                }
                filming.at += 1;
                filming.stage = if filming.at < filming.pages.len() {
                    Stage::Standing(0)
                } else {
                    Stage::Draining
                };
            }
            Stage::Filming(frame) => {
                drive(world, &filming.slots);
                aim(world, filming, frame as f32 / frames_of(REVOLUTION) as f32);
                let readback = world
                    .spawn(ReadbackOnce::texture(filming.picture.clone()))
                    .id();
                filming.film.pending.push_back(readback);
                filming.stage = Stage::Filming(frame + 1);
            }
            Stage::Draining => {
                if filming.film.pending.is_empty() {
                    filming.film.finish();
                    world.write_message(AppExit::Success);
                }
            }
        }
    });
}

fn frames_of(seconds: Seconds) -> u32 {
    (seconds.0 * FPS as f32).round() as u32
}

/// Stands `tile` at the place of `slot`, its vfx playing on it.
fn stand(world: &mut World, content: &Content, slot: usize, tile: &Staged) -> Shown {
    let at = Page::spot(slot);
    let feet = world_point(at, ground_height(world, at));
    let placed = Transform::from_translation(feet);
    let (thing, body) = match &tile.on {
        CoatCarrier::Body { model, held, .. } => {
            let posed = Posed {
                model,
                held,
                clip: None,
                moment: None,
            };
            let body = body::stand(world, content, &posed, placed);
            let hitbox = body.model(content).hitbox();
            world.entity_mut(body.rig).insert((
                SiteExtent {
                    footprint: Vec2::splat(hitbox.radius.0 * 2.0),
                    height: hitbox.height.0,
                },
                StandingVfx::playing(vec![tile.play.clone()]),
            ));
            (body.rig, Some(body))
        }
        CoatCarrier::Prop { prop } => {
            let prop: PropId = content.by_path::<PropDef>(prop).expect("a prop found");
            let model = prop.get(content).model.as_str();
            let extent = content.built::<GltfInfos>().0.get(model).map_or(
                SiteExtent {
                    footprint: Vec2::ONE,
                    height: 1.0,
                },
                |info| scenery::extent_of(info, 1.0),
            );
            let thing = world
                .spawn((
                    placed,
                    Visibility::default(),
                    extent,
                    ShowsProp(prop),
                    StandingVfx::playing(vec![tile.play.clone()]),
                ))
                .id();
            (thing, None)
        }
        CoatCarrier::Ground { shape } => {
            let now = server_now(world).expect("the stage's clock set");
            let thing = world
                .spawn(VfxInstance {
                    vfx: tile.play.vfx,
                    site: VfxSite::Area(PlacedShape {
                        at,
                        facing: FRONT,
                        shape: *shape,
                    }),
                    part: VfxPart::Whole,
                    params: tile.play.params(),
                    dealer: None,
                    heading: None,
                    started: now,
                    until: None,
                })
                .id();
            (thing, None)
        }
    };
    let (framing, measured) = match &tile.on {
        CoatCarrier::Ground { shape } => {
            let (ahead, radius) = reach(*shape);
            (Framing::round(feet + Vec3::Z * ahead, radius), true)
        }
        CoatCarrier::Body { .. } | CoatCarrier::Prop { .. } => (
            Framing::round(feet + Vec3::Y * STANDING_IN.0, STANDING_IN.1),
            false,
        ),
    };
    Shown {
        label: tile.label.clone(),
        thing,
        body,
        framing,
        measured,
    }
}

impl Shown {
    /// Whether the tile stands whole: its body hung and given the hand it deals from, and every
    /// model under it shown.
    fn stood(&mut self, world: &mut World, content: &Content) -> bool {
        if let Some(body) = &mut self.body {
            if !body.hang(world, content) {
                return false;
            }
            if world.get::<VfxHand>(body.rig).is_none() {
                let hand = body
                    .model(content)
                    .hands()
                    .first()
                    .map(|hand| hand.joint.0.clone());
                let joint = hand.and_then(|hand| {
                    world
                        .get::<world_body::Body>(body.rig)
                        .and_then(|posed| posed.joints().get(&hand).copied())
                });
                if let Some(joint) = joint {
                    world.entity_mut(body.rig).insert(VfxHand(joint));
                }
            }
        }
        let mut pending = vec![self.thing];
        while let Some(at) = pending.pop() {
            if world.get::<ShowsProp>(at).is_some() && world.get::<WorldAssetRoot>(at).is_none() {
                return false;
            }
            if let Some(children) = world.get::<Children>(at) {
                pending.extend(children.iter());
            }
        }
        true
    }

    /// Frames the camera round the tile's thing as it stands now, turning about the middle of its
    /// own meshes: everything it is drawn with, what it holds included, whichever way it turns.
    fn measure(&mut self, world: &World) {
        if self.measured {
            return;
        }
        self.measured = true;
        let mut own = Vec::new();
        let mut held = Vec::new();
        for (at, in_held) in drawn_with_in(world, self.thing) {
            if let (Some(aabb), Some(global)) =
                (world.get::<Aabb>(at), world.get::<GlobalTransform>(at))
            {
                let (c, h) = (Vec3::from(aabb.center), Vec3::from(aabb.half_extents));
                let corners = (0..8).map(|k| {
                    global.transform_point(
                        c + h * Vec3::new(
                            if k & 1 == 0 { -1.0 } else { 1.0 },
                            if k & 2 == 0 { -1.0 } else { 1.0 },
                            if k & 4 == 0 { -1.0 } else { 1.0 },
                        ),
                    )
                });
                if in_held {
                    held.extend(corners);
                } else {
                    own.extend(corners);
                }
            }
        }
        let middle = |points: &[Vec3]| {
            let lo = points.iter().copied().reduce(Vec3::min)?;
            let hi = points.iter().copied().reduce(Vec3::max)?;
            Some((lo + hi) / 2.0)
        };
        let Some(center) = middle(&own).or_else(|| middle(&held)) else {
            return;
        };
        let radius = own
            .iter()
            .chain(&held)
            .map(|corner| corner.distance(center))
            .fold(0.0, f32::max);
        self.framing = Framing::round(center, radius);
    }
}

/// How far ahead of its point a shape laid facing [`FRONT`] reaches out its middle, and how far
/// round that its farthest edge lies.
fn reach(shape: Shape) -> (f32, f32) {
    match shape {
        Shape::Circle { radius } => (0.0, radius.0),
        Shape::Rect { width, height } => (0.0, width.0.hypot(height.0) / 2.0),
        Shape::Cone { length, .. } => (0.0, length.0),
        Shape::Line { length, width } => (length.0 / 2.0, length.0.hypot(width.0) / 2.0),
    }
}

/// Drives the layers of every body on the page.
fn drive(world: &mut World, slots: &[Slot]) {
    body::drive(
        world,
        slots
            .iter()
            .filter_map(|slot| slot.shown.as_ref()?.body.as_ref()),
    );
}

/// Shows every tile turned `turned` of the way round: each camera and the sun gone round it the
/// other way, about the vertical through its middle. A camera with no tile rests.
fn aim(world: &mut World, filming: &Filming, turned: f32) {
    let round = Quat::from_rotation_y(-turned * TAU);
    if let Some(mut sun) = world.get_mut::<Transform>(filming.sun) {
        *sun = lights::sun_transform(
            SUN_ELEVATION,
            Degrees(SUN_AZIMUTH.0 + turned * 360.0),
            Vec2::ZERO,
        );
    }
    let page = filming.page;
    for slot in &filming.slots {
        let Some(shown) = &slot.shown else {
            if let Some(mut camera) = world.get_mut::<Camera>(slot.camera) {
                camera.is_active = false;
            }
            continue;
        };
        let framing = shown.framing;
        let mut camera = world.entity_mut(slot.camera);
        camera.insert((
            framing.eye(round),
            framing.lens(page.tile.width, page.view_height()),
            CameraFocus(framing.center),
        ));
        if let Some(mut active) = camera.get_mut::<Camera>()
            && !active.is_active
        {
            active.is_active = true;
        }
    }
}

/// Lays the page out: each tile's picture in its place, row by row, and its label in a bar under
/// it.
fn draw(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let filming = world.resource::<Filming>();
    let page = filming.page;
    let painter = gui.layer_painter(LayerId::background());
    let pixels = gui.pixels_per_point();
    let (width, height) = (page.tile.width as f32, page.tile.height as f32);
    let view = page.view_height() as f32;
    let font = FontId::proportional(page.bar() as f32 * 0.55 / pixels);
    for (index, slot) in filming.slots.iter().enumerate() {
        let Some(shown) = &slot.shown else {
            continue;
        };
        let left = (index as u32 % page.columns) as f32 * width;
        let top = (index as u32 / page.columns) as f32 * height;
        let points = |x: f32, y: f32| Pos2::new(x / pixels, y / pixels);
        let picture = Rect::from_min_max(points(left, top), points(left + width, top + view));
        painter.image(
            slot.texture,
            picture,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
        let bar = Rect::from_min_max(points(left, top + view), points(left + width, top + height));
        painter.rect_filled(bar, 0.0, BAR_FILL);
        painter.text(
            bar.center(),
            Align2::CENTER_CENTER,
            &shown.label,
            font.clone(),
            LABEL_INK,
        );
    }
}
