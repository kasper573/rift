mod editor;
mod frames;
mod game;
mod hitches;
pub mod socket;

pub use editor::{
    EditorFrameQuestion, EditorPanel, EditorScenery, EditorSnapshot, EditorStage, EditorStageMark,
    EditorTree, EditorTreeRow, EditorTrees,
};

pub use frames::{
    FRAMES_KEPT, FrameSample, FrameSamples, HITCH, LONGEST_SYSTEMS, MissingParts, ProbeFramePasses,
    ProbeFrames, ProbePass, ProbeSlowFrame, ProbeSystem, SETTLE_FRAMES, SLOW_KEPT,
};
pub use game::{
    ProbeArea, ProbeBinding, ProbeBindings, ProbeButton, ProbeFrameQuestion, ProbeGame,
    ProbeGameQuestion, ProbeGesture, ProbeInterfaceIcon, ProbeItem, ProbeNamedObject, ProbeNpc,
    ProbeProp, ProbeResident, ProbeSkill, ProbeSpawner, ProbeWater, answer,
};
pub use hitches::{SystemTimes, timing_layer};

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use ui::egui;
use ui::float;

use bevy::diagnostic::FrameCount;
use bevy::input::ButtonState;
use bevy::input::keyboard::KeyboardInput;
use bevy::input::mouse::{MouseButtonInput, MouseWheel};
use bevy::prelude::*;
use bevy::render::renderer::{RenderAdapterInfo, RenderDevice};
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::tasks::AsyncComputeTaskPool;
use bevy::window::{CursorOptions, PrimaryWindow};
use bevy_egui::{EguiOutput, EguiPostUpdateSet, PrimaryEguiContext};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::audio::Hearing;
use crate::core::audio::mix::AudioCategory;
use crate::core::audio::playback::Listener;
use crate::core::audio::soundscape::{self, SoundscapeMixer, SoundscapeShape};
use crate::core::chunk::Chunk;
use crate::core::content::{Content, ContentScope, with_content};
use crate::core::geometry::Capsule;
use crate::core::gui::{self, GuiContext, WorldLayers};
use crate::core::interpolate::Jumps;
use crate::core::math::{
    MatrixSchema, Offset, OffsetSchema, Pos, PosSchema, Rect, RectSchema, Size, SizeSchema,
};
use crate::core::render::sample::{
    self, FrameSamplingPlugin, FrameWork, GpuReportsRead, RenderReports,
};
use crate::core::render::transition::{ActiveTransition, ScreenTransitionPhase};
use crate::core::render::video::VideoPreferences;
use crate::core::render::world::clouds::{CloudDrift, CloudSheet};
use crate::core::render::world::layers::Layered;
use crate::core::render::world::sky::Daylight;
use crate::core::render::world::{OnItsWay, missing_drawn};
use crate::core::render::{self, CameraRig, GroundHeights, Pointed, ground_height, world_point};
use crate::core::time;
use crate::core::units::{
    Bytes, MeterSize, Meters, Millis, Percent, PlaybackRate, Radians, Seconds, WindowPx,
};
use crate::core::vfx::cover::ShownWeather;
use crate::core::vfx::rain::{RainField, Shelter};
use crate::core::vfx::{VfxId, VfxInstance, VfxSite};
use crate::systems::actor::held::{self, Held};
use crate::systems::actor::render::{Hung, Puppet, RenderActor};
use crate::systems::actor::skeleton::{Clip, Naming, SkeletonId};
use crate::systems::actor::stance::{HandIndex, StanceId};
use crate::systems::actor::{
    self, Action, Actor, ActorModel, Appearance, Hitbox, Motion, Playing, Pose,
};
use crate::systems::area::weather::ShownCourse;
use crate::systems::area::{self, AreaId, AreaObject, AreaTag, PropId};
use crate::systems::attention::{Attention, AttentionId, StatusBadges};
use crate::systems::combat::Attitude;
use crate::systems::crafting;
use crate::systems::dialogue::camera::DialogueCamera;
use crate::systems::dialogue::stage;
use crate::systems::equipment::{self, EquipmentPlace};
use crate::systems::history::widget::{self as history, HistoryBook};
use crate::systems::history::{HistoryMark, HistoryRecord, HistoryTopic};
use crate::systems::hud;
use crate::systems::input::claims::{self, ClickClaim};
use crate::systems::input::map::InputMap;
use crate::systems::interface::CursorId;
use crate::systems::item::{self, DroppedItem, Inventory, ItemId, ItemStack};
use crate::systems::movement::{Position, RenderPosition};
use crate::systems::notification::{self, NotificationRows, bubble, error, feed, intro, pop};
use crate::systems::npc::{Npc, NpcId};
use crate::systems::player::{PlayedBy, commands_locked, session};
use crate::systems::prompt;
use crate::systems::quest::{Parties, QuestId, QuestLog, QuestResult};
use crate::systems::reach::Engaged;
use crate::systems::resource::{self, Left, Resource, ResourceId, Working};
use crate::systems::script::{EffectReach, OwnerRow, RiftScripts};
use crate::systems::shop::{self, ShopId};
use crate::systems::skill::{self, Rank, SkillId, bar::SlotReport};
use crate::systems::stat::{StatKind, Stats};
use crate::systems::status::{self, StatusCategory, StatusEffectId};

/// The probe's question about the game `line`, read against the game it names places of.
pub fn read_question(content: &Content, line: &str) -> Result<ProbeGameQuestion, String> {
    let (question, scope) = with_content(ContentScope::of(content), || {
        serde_json::from_str::<ProbeGameQuestion>(line)
    });
    let question = question.map_err(|error| error.to_string())?;
    match scope
        .leaves()
        .iter()
        .find_map(crate::core::content::id::Leaf::reported)
    {
        Some(problem) => Err(problem.to_string()),
        None => Ok(question),
    }
}

/// The JSON Schema of the probe's protocol: what a driver asks, and each kind of answer.
pub fn schema() -> String {
    let schema = schemars::schema_for!(ProbeProtocol);
    serde_json::to_string_pretty(&schema).expect("a serializable schema")
}

/// Everything the probe speaks, for its schema: the questions of each kind, and the answers of
/// each kind, the client's and the editor's.
#[derive(JsonSchema)]
pub struct ProbeProtocol {
    pub frame_question: ProbeFrameQuestion,
    pub game_question: ProbeGameQuestion,
    pub snapshot: ProbeSnapshot,
    pub game: ProbeGame,
    #[schemars(with = "bool")]
    pub yes: bool,
    #[schemars(with = "Option<PosSchema>")]
    pub point: Option<Pos<Meters>>,
    #[schemars(with = "Option<Vec<PosSchema>>")]
    pub route: Option<Vec<Pos<Meters>>>,
    #[schemars(with = "Vec<PosSchema>")]
    pub points: Vec<Pos<Meters>>,
    pub water: ProbeWater,
    pub batch: Vec<serde_json::Value>,
    pub frames: ProbeFrames,
    pub editor_question: EditorFrameQuestion,
    pub editor_snapshot: EditorSnapshot,
    pub editor_trees: EditorTrees,
}

/// Answers the driver at `driver`: the frame after it asks for a snapshot answers it, and every
/// other question is about the game, answered on its own once the game is loaded.
pub fn install(app: &mut App, driver: std::net::SocketAddr) {
    let game: Arc<(Mutex<Option<Content>>, Condvar)> = Arc::default();
    let socket = socket::ProbeSocket::connect(driver, {
        let game = game.clone();
        move |line| {
            let content = loaded(&game);
            match read_question(&content, line) {
                Ok(question) => answer(&content, &question),
                Err(error) => {
                    bevy::log::error!("the probe's driver asked {line}: {error}");
                    std::process::exit(1);
                }
            }
        }
    });
    app.insert_non_send(Probe { socket, game })
        .add_plugins((ProbeFramesPlugin, ProbeInterfacePlugin))
        .init_resource::<TerrainSent>()
        .init_resource::<VfxSeen>()
        .init_resource::<FrameTimes>()
        .add_observer(remember_vfx)
        .add_systems(PreUpdate, read_accessibly)
        .add_systems(
            PostUpdate,
            count_clicks.after(EguiPostUpdateSet::ProcessOutput),
        )
        .add_systems(Last, (time_frame, publish).chain().after(sample_frame));
}

/// Samples every frame into [`FrameSamples`]: how long the main world, the render world and the
/// GPU took over it, and what it drew.
pub struct ProbeFramesPlugin;

impl Plugin for ProbeFramesPlugin {
    fn build(&self, app: &mut App) {
        if let Some(times) = app.world().get_resource::<SystemTimes>() {
            times.listen();
        }
        app.add_plugins(FrameSamplingPlugin)
            .init_resource::<FrameSamples>()
            .init_resource::<FrameMark>()
            .add_systems(First, mark_frame)
            .add_systems(
                Last,
                sample_frame.after(bevy::diagnostic::update_frame_count),
            );
    }
}

/// Counts the input the app received into [`ProbeInputSeen`], and keeps the accessibility tree
/// the primary interface last drew in [`DrawnUi`]; the app turns the tree on.
pub struct ProbeInterfacePlugin;

impl Plugin for ProbeInterfacePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ProbeInputSeen>()
            .init_resource::<DrawnUi>()
            .add_systems(PreUpdate, count_input)
            .add_systems(
                PostUpdate,
                copy_drawn_ui.after(EguiPostUpdateSet::ProcessOutput),
            );
    }
}

/// How long each frame took since the last snapshot.
#[derive(Resource, Default)]
struct FrameTimes(std::collections::VecDeque<Millis>);

fn time_frame(time: Res<Time<Real>>, mut times: ResMut<FrameTimes>) {
    if times.0.len() == FRAMES_KEPT {
        times.0.pop_front();
    }
    times.0.push_back(Millis(time.delta_secs() * 1000.0));
}

/// When the main world began the frame.
#[derive(Resource, Default)]
struct FrameMark(Option<std::time::Instant>);

fn mark_frame(mut mark: ResMut<FrameMark>) {
    mark.0 = Some(std::time::Instant::now());
}

/// Keeps the frame's sample, and fills in what the render world and the GPU have reported of
/// earlier frames since the last.
pub fn sample_frame(world: &mut World) {
    let frame = world.resource::<FrameCount>().0;
    let dt_ms = Millis(world.resource::<Time<Real>>().delta_secs() * 1000.0);
    let cpu_main_ms = world
        .resource::<FrameMark>()
        .0
        .map_or(Millis(0.0), |began| {
            Millis(began.elapsed().as_secs_f32() * 1000.0)
        });
    let shortfalls = area::scenery::shortfalls(world);
    let drawn_coarser = missing_drawn(world);
    let missing = shortfalls.total().missing + drawn_coarser;
    let missing_parts = MissingParts {
        ground: shortfalls.ground.missing,
        props: shortfalls.props.missing,
        copies: shortfalls.copies.missing,
        far: shortfalls.far.missing,
        canopy: shortfalls.canopy.missing,
        grass: shortfalls.grass.missing,
        drawn: drawn_coarser,
    };
    let stale = shortfalls.total().stale;
    let visible_chunks = area::scenery::stood_chunks(world);
    let entities = world.entities().len();
    let work = world
        .get_resource_mut::<FrameWork>()
        .map(|mut work| std::mem::take(&mut *work))
        .unwrap_or_default();
    let systems = world
        .get_resource::<SystemTimes>()
        .map(|times| times.take_longest(frames::LONGEST_SYSTEMS))
        .unwrap_or_default();
    let drawn = world.resource::<RenderReports>().take();
    let timed = world.resource_scope(|world, mut read: Mut<GpuReportsRead>| {
        world
            .get_resource::<bevy::diagnostic::DiagnosticsStore>()
            .map(|store| sample::gpu_reports(store, &mut read))
            .unwrap_or_default()
    });
    let mut samples = world.resource_mut::<FrameSamples>();
    samples.push(
        FrameSample {
            frame,
            dt_ms,
            cpu_main_ms,
            cpu_render_ms: None,
            gpu_ms: None,
            props_drawn: None,
            missing,
            missing_parts,
            stale,
            draws: 0,
            instances: 0,
            entities,
            visible_chunks,
            upload_bytes: Bytes::default(),
            vfx_writes: work.vfx_writes,
            posed: work.posed,
            plans: work.plans,
        },
        systems,
    );
    for report in &drawn {
        samples.drawn(report);
    }
    for report in &timed {
        samples.timed(report);
    }
}

/// One frame of the client. Content ids (de)serialize by key, so a driver reads a snapshot within a
/// content scope. World positions are metres on the ground, which `view` and `terrain` place in the
/// window; window positions are the window's logical pixels.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeSnapshot {
    pub frame: u32,
    pub input: ProbeInput,
    pub claim: ClickClaim,
    #[schemars(with = "Option<SizeSchema>")]
    pub screen: Option<Size<WindowPx>>,
    pub view: Option<ProbeView>,
    pub area: Option<AreaId>,
    /// The ground's heights around the player: the chunks within the view's reach, sent as the
    /// player comes to new ground and left out while they stay, so a driver keeps the last it saw.
    pub terrain: Option<ProbeTerrain>,
    pub me: Option<ProbeBody>,
    pub actors: Vec<ProbeBody>,
    pub objects: Vec<ProbeObject>,
    pub items: Vec<ProbeGroundItem>,
    pub resources: Vec<ProbeResource>,
    pub warps: Vec<ProbeWarp>,
    /// What the interface shows: text, icons, fields and sliders, less what is scrolled or clipped
    /// out of sight.
    pub ui: Vec<ProbeUiElement>,
    /// Where the interface takes the pointer: each rect edges included, as egui hit-tests.
    #[schemars(with = "Vec<RectSchema>")]
    pub covered: Vec<Rect<WindowPx>>,
    pub stage: Option<ProbeStage>,
    pub notifications: ProbeNotifications,
    pub feed: Vec<String>,
    pub feed_place: feed::FeedPlace,
    pub pops: Vec<pop::ShownPop>,
    pub history: ProbeHistory,
    pub shop: Option<ProbeShop>,
    pub prompt: Option<ProbePrompt>,
    pub crafting: Option<ProbeCrafting>,
    pub item_card: Option<ProbeItemCard>,
    pub skill_card: Option<ProbeSkillCard>,
    pub bag: Vec<ProbeStack>,
    pub quests: ProbeQuests,
    pub hotbar: Vec<SlotReport>,
    pub statuses: Vec<ProbeStatus>,
    pub vfx: Vec<ProbeVfx>,
    pub basic_attack: Option<SkillId>,
    pub skills: Option<ProbeSkills>,
    pub soundscape: ProbeSoundscape,
    pub weather: Option<ProbeWeather>,
    pub transition: ScreenTransitionPhase,
    pub daylight: Daylight,
    /// What the GPU holds in buffers and textures for the client.
    pub gpu_memory: Bytes,
    /// How long each frame took since the previous snapshot, the latest `FRAMES_KEPT` of them.
    pub frame_times: Vec<Millis>,
    /// Whether the scenery has yet to stand all the view takes in: until it has, some of what the
    /// view shows is missing.
    pub scenery_standing: bool,
    /// How many sceneries have yet to stand what their sight takes in.
    pub sceneries_standing: u32,
    /// What the world draws that is still on its way to standing.
    pub on_its_way: OnItsWay,
    /// The graphics adapter the client draws with, by the name its driver gives it.
    pub adapter: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug)]
pub struct ProbeInput {
    #[schemars(with = "Option<PosSchema>")]
    pub cursor: Option<Pos<WindowPx>>,
    /// Whether the system draws the cursor, rather than the frame or nothing.
    pub system_cursor: bool,
    /// The cursor of the game's the pointer shows in the area, none while it shows the system's.
    pub game_cursor: Option<CursorId>,
    /// Whether a click now would still count on from the latest one, as a double or triple click.
    pub click_run_open: bool,
    #[serde(flatten)]
    pub seen: ProbeInputSeen,
}

/// What the driver has sent so far, as the game received it: a driver that counts what it sends
/// knows when the game has caught up, however slowly frames come.
#[derive(
    Resource, Serialize, Deserialize, JsonSchema, Default, Clone, Copy, PartialEq, Eq, Debug,
)]
pub struct ProbeInputSeen {
    pub mouse_presses: u32,
    pub mouse_releases: u32,
    /// How many clicks in a row the game counted the latest one as.
    pub click_run: u8,
    pub wheel_turns: u32,
    pub key_presses: u32,
}

/// The camera: how a world point (x east, height up, y south) lands in the window, and where the
/// camera looks from.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug)]
pub struct ProbeView {
    #[schemars(with = "MatrixSchema")]
    pub view_proj: Mat4,
    #[schemars(with = "RectSchema")]
    pub viewport: Rect<WindowPx>,
    pub yaw: Radians,
    pub pitch: Radians,
    pub zoom: u8,
    /// Whether it rests where going home puts it: facing north, at its usual tilt and distance.
    pub home: bool,
}

impl ProbeView {
    /// Where a point `height` above the ground at `at` shows in the window, or beyond its edges.
    /// A point beside or behind the camera shows nowhere.
    pub fn window_point(&self, at: Pos<Meters>, height: Meters) -> Option<Pos<WindowPx>> {
        let clip = self.view_proj * Vec4::new(at.x, height.0, at.y, 1.0);
        if clip.w <= f32::EPSILON {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        Some(
            self.viewport.origin
                + Offset::new(
                    (ndc.x + 1.0) / 2.0 * self.viewport.size.width,
                    (1.0 - ndc.y) / 2.0 * self.viewport.size.height,
                ),
        )
    }
}

/// Ground heights a metre apart, row by row from the corner at `origin` on: `size` plus one of
/// them to a row, and as many rows. Beyond its edge the nearest height stands in.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ProbeTerrain {
    #[schemars(with = "PosSchema")]
    pub origin: Pos<Meters>,
    #[schemars(with = "SizeSchema")]
    pub size: Size<Meters>,
    pub heights: Vec<Meters>,
}

impl ProbeTerrain {
    pub fn height_at(&self, at: Pos<Meters>) -> Meters {
        let columns = self.size.width as usize + 1;
        let rows = self.size.height as usize + 1;
        let x = (at.x - self.origin.x).clamp(0.0, columns as f32 - 1.0);
        let y = (at.y - self.origin.y).clamp(0.0, rows as f32 - 1.0);
        let (column, row) = (x.floor() as usize, y.floor() as usize);
        let (u, v) = (x - column as f32, y - row as f32);
        let sample = |column: usize, row: usize| {
            self.heights[row.min(rows - 1) * columns + column.min(columns - 1)].0
        };
        let top = sample(column, row) + (sample(column + 1, row) - sample(column, row)) * u;
        let bottom =
            sample(column, row + 1) + (sample(column + 1, row + 1) - sample(column, row + 1)) * u;
        Meters(top + (bottom - top) * v)
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeBody {
    #[schemars(with = "u64")]
    pub id: Entity,
    pub name: String,
    pub appearance: Appearance,
    pub flies: bool,
    pub player: bool,
    #[schemars(with = "PosSchema")]
    pub at: Pos<Meters>,
    /// Where it shows on screen, which runs behind `at` for others and ahead of it for the player's
    /// own walk.
    #[schemars(with = "PosSchema")]
    pub drawn: Pos<Meters>,
    /// Where a click takes it rather than whatever stands in front of it.
    #[schemars(with = "PosSchema")]
    pub aim: Pos<Meters>,
    pub capsule: Capsule,
    /// The way its body is turned on screen.
    pub facing: Radians,
    pub action: Action,
    /// The pose it holds while it idles.
    pub pose: Pose,
    /// The statuses it carries.
    pub statuses: Vec<StatusEffectId>,
    /// The actor it is casting at, a blow or a bolt alike.
    #[schemars(with = "Option<u64>")]
    pub engaged: Option<Entity>,
    /// Whether it stands still rather than walks.
    pub resting: bool,
    /// How many times it was put where it stands rather than walked there: a warp, a respawn.
    #[schemars(with = "u32")]
    pub jumps: Jumps,
    /// The motion its body shows: what it does, or a move, a gesture, a flinch at a blow that
    /// just landed on it.
    pub shown: Motion,
    /// Where its feet and the top of its head show in the window, as drawn: nowhere, behind the
    /// camera.
    #[schemars(with = "Option<PosSchema>")]
    pub feet: Option<Pos<WindowPx>>,
    #[schemars(with = "Option<PosSchema>")]
    pub head: Option<Pos<WindowPx>>,
    pub health: f32,
    pub max_health: f32,
    pub npc: Option<NpcId>,
    pub role: Option<String>,
    pub friendly: bool,
    pub locked: bool,
    pub marks: Vec<AttentionId>,
    pub badges: Vec<AttentionId>,
    pub skeleton: SkeletonId,
    /// The stance its hands or a script put it in.
    pub stance: Option<StanceId>,
    /// The clip it plays for the motion it shows, as its stance resolves it, and the motion it was
    /// told to play, if any.
    pub clip: Clip,
    pub motion: Option<Motion>,
    /// How fast that clip plays: a hit at its attack speed, a gait as fast as covers its speed.
    pub rate: PlaybackRate,
    /// Where its skeleton's sets named that clip.
    pub naming: Naming,
    /// What its skeleton calls each of its hands, in their order.
    pub hands: Vec<String>,
    /// What each hand of its body shows, as the server says.
    pub wearing: Vec<ProbeWorn>,
    /// The item in its tool slot.
    pub tool: Option<ItemId>,
    /// The work it has under way at a resource.
    pub gathering: Option<ProbeGathering>,
    /// The props its rig holds as drawn, each where it shows in the window.
    pub held: Vec<ProbeHeld>,
    /// Whether any layer moves its joints this frame.
    pub layered: bool,
}

/// A prop hanging from a hand of a drawn body.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeWorn {
    pub hand: HandIndex,
    pub item: ItemId,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeHeld {
    pub hand: HandIndex,
    /// The item it is, when the body holds it as one.
    pub item: Option<ItemId>,
    /// The prop it is, when it is the tool of the work the body's hands are at.
    pub tool: Option<PropId>,
    #[schemars(with = "Option<PosSchema>")]
    pub at: Option<Pos<WindowPx>>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeObject {
    #[schemars(with = "u64")]
    pub id: Entity,
    pub name: Option<String>,
    pub title: Option<String>,
    #[schemars(with = "PosSchema")]
    pub at: Pos<Meters>,
    #[schemars(with = "PosSchema")]
    pub aim: Pos<Meters>,
    #[schemars(with = "RectSchema")]
    pub hitbox: Rect<Meters>,
    pub marks: Vec<AttentionId>,
    /// The statuses it carries.
    pub statuses: Vec<StatusEffectId>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeGroundItem {
    #[schemars(with = "u64")]
    pub id: Entity,
    pub item: ItemId,
    pub count: u32,
    #[schemars(with = "PosSchema")]
    pub at: Pos<Meters>,
}

/// A resource standing in the world.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeResource {
    #[schemars(with = "u64")]
    pub id: Entity,
    pub resource: ResourceId,
    pub name: String,
    #[schemars(with = "PosSchema")]
    pub at: Pos<Meters>,
    /// Where a click takes it rather than whatever stands in front of it.
    #[schemars(with = "PosSchema")]
    pub aim: Pos<Meters>,
    pub capsule: Capsule,
    /// How many extractions it still holds.
    pub left: u32,
    /// How many may work it at once.
    pub room: u32,
    /// Who works it.
    #[schemars(with = "Vec<u64>")]
    pub workers: Vec<Entity>,
}

/// The work a character has under way at a resource.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeGathering {
    #[schemars(with = "u64")]
    pub resource: Entity,
    /// The tool it works with, none by hand.
    pub tool: Option<ItemId>,
    /// How far through the extraction under way it is, from none to all of it.
    pub progress: f32,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeWarp {
    pub name: String,
    pub to: AreaId,
    #[schemars(with = "PosSchema")]
    pub at: Pos<Meters>,
    #[schemars(with = "RectSchema")]
    pub rect: Rect<Meters>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, PartialEq, Debug)]
pub struct ProbeUiElement {
    pub text: Option<String>,
    pub image: Option<String>,
    pub editable: bool,
    pub button: bool,
    pub focused: bool,
    pub click_through: bool,
    pub in_world: bool,
    pub slider: Option<f32>,
    /// Whether a checkbox is ticked; none for anything else.
    pub checked: Option<bool>,
    /// The text of the label that names the element, as a form's label names the field beside it.
    pub labelled_by: Option<String>,
    #[schemars(with = "RectSchema")]
    pub rect: Rect<WindowPx>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeStage {
    pub node: String,
    /// The dialogue box as last drawn.
    #[schemars(with = "Option<RectSchema>")]
    pub rect: Option<Rect<WindowPx>>,
    #[schemars(with = "Option<u64>")]
    pub with: Option<Entity>,
    /// The camera the line on show is seen through.
    pub camera: DialogueCamera,
    pub line: usize,
    pub lines: usize,
    pub speaker: Option<String>,
    pub text: Option<String>,
    pub typing: bool,
    pub choices: Vec<ProbeChoice>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeChoice {
    pub label: String,
    /// The items choosing it gives.
    pub gets: Vec<ItemId>,
    pub locked: bool,
    pub selected: bool,
    #[schemars(with = "Option<RectSchema>")]
    pub rect: Option<Rect<WindowPx>>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeNotifications {
    pub bubbles: Vec<bubble::ShownBubble>,
    pub edge_folds: Vec<u32>,
    pub captions: NotificationRows,
    pub error: Option<String>,
    pub milestones: NotificationRows,
    pub intro: Option<intro::ShownIntro>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeHistory {
    pub open: bool,
    pub tab: Option<String>,
    pub records: Vec<ProbeRecord>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeRecord {
    pub topic: HistoryTopic,
    pub by: Option<String>,
    pub text: String,
    pub mark: Option<HistoryMark>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeShop {
    pub shop: ShopId,
    pub offers: Vec<ProbeOffer>,
    pub buyback: Vec<ProbeStack>,
}

/// The prompt before the player and what they have placed in its tray.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbePrompt {
    pub title: String,
    pub text: Option<String>,
    pub slots: u32,
    pub placed: Vec<ProbePlaced>,
    /// Whether what is placed was submitted and the server has yet to answer.
    pub waiting: bool,
}

/// What the prompt before the player crafts: every recipe it was opened with, what the items
/// placed make, and what its button says.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeCrafting {
    pub recipes: Vec<ProbeRecipe>,
    /// Whether the window listing its recipes is open.
    pub listing: bool,
    /// What the items placed make, one result for each recipe they are a whole number of times:
    /// everything that recipe would give for them.
    pub results: Vec<Vec<ProbeStack>>,
    /// Which of the results its button would make: the only one, or of several the one chosen.
    pub picked: Option<usize>,
    pub button: String,
    /// Whether the button can be pressed.
    pub ready: bool,
    /// How far through the craft under way it is, from none to all of it.
    pub progress: Option<f32>,
}

/// A recipe of a crafting prompt, as its window lists it.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeRecipe {
    /// Everything one craft gives.
    pub makes: Vec<ProbeStack>,
    pub from: Vec<ProbeStack>,
    /// How long one craft takes at a crafting speed of 100%.
    pub takes: Seconds,
    /// How often one craft comes off: one that fails spends its materials and makes nothing.
    pub chance: Percent,
    /// Whether the player's bag covers every material.
    pub can_make: bool,
}

/// A stack placed in a prompt's tray.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbePlaced {
    pub item: ItemId,
    pub count: u32,
    /// Whether the bag no longer covers what was placed.
    pub short: bool,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeOffer {
    pub item: ItemId,
    pub count: u32,
    pub left: Option<u32>,
    pub refusal: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug)]
pub struct ProbeStack {
    pub item: ItemId,
    pub count: u32,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug)]
pub struct ProbeItemCard {
    pub item: ItemId,
    #[schemars(with = "RectSchema")]
    pub rect: Rect<WindowPx>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug)]
pub struct ProbeSkillCard {
    pub skill: SkillId,
    #[schemars(with = "RectSchema")]
    pub rect: Rect<WindowPx>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ProbeStatus {
    pub effect: StatusEffectId,
    pub category: StatusCategory,
    pub left: Option<Seconds>,
}

/// A vfx seen starting lately, with the site it plays on; `over` once its time has run out. One
/// shorter than the gap between two snapshots is still reported in the next.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ProbeVfx {
    pub vfx: VfxId,
    pub site: VfxSite,
    pub over: bool,
}

/// The character's class, tree and bar as the client shows them.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ProbeSkills {
    pub class: String,
    pub level: u32,
    pub to_spend: u32,
    pub learned: Vec<ProbeRank>,
    pub planned: Vec<ProbeRank>,
    pub aiming: Option<SkillId>,
    pub reset_prompt: bool,
    /// The skills window, while open.
    pub view: Option<ProbeTreeView>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ProbeRank {
    pub skill: String,
    pub rank: Rank,
}

/// How the skills window shows the tree: the canvas (backdrop and nodes) pans within the
/// viewport, which the canvas always fills, and the window stops growing at `max`.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ProbeTreeView {
    #[schemars(with = "RectSchema")]
    pub window: Rect<WindowPx>,
    #[schemars(with = "Option<SizeSchema>")]
    pub max: Option<Size<WindowPx>>,
    #[schemars(with = "RectSchema")]
    pub viewport: Rect<WindowPx>,
    #[schemars(with = "SizeSchema")]
    pub canvas: Size<WindowPx>,
    #[schemars(with = "OffsetSchema")]
    pub panned: Offset<WindowPx>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default)]
pub struct ProbeQuests {
    pub active: Vec<ProbeQuest>,
    pub finished: Vec<ProbeFinishedQuest>,
    pub tracked: Vec<QuestId>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeQuest {
    pub quest: QuestId,
    pub ready: bool,
    pub left: Option<Seconds>,
    /// Each objective's [have, need].
    pub progress: Vec<[u32; 2]>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug)]
pub struct ProbeFinishedQuest {
    pub quest: QuestId,
    pub result: QuestResult,
}

/// The weather over the shown area as the client draws it: the spell under way by its name, or
/// null before the course is known; how much cloud covers the sky, how much water it holds and how
/// hard it rains under the wettest of it, each 0 to 1; and where the player stands, how hard it
/// rains, how wet the ground lies, whether they stand sheltered from the rain, and how much of the
/// sun's or the moon's light the cloud overhead takes from them, 0 none to 1 all of it; and how
/// brightly lightning lights the world this moment.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeWeather {
    pub spell: Option<String>,
    pub clouds: f32,
    pub water: f32,
    pub rain: f32,
    pub raining: f32,
    pub wetness: f32,
    pub sheltered: bool,
    pub shaded: f32,
    pub flash: f32,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeSoundscape {
    pub zones: Vec<ProbeSoundZone>,
    pub voices: Vec<ProbeVoice>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeSoundZone {
    #[schemars(with = "RectSchema")]
    pub bounds: Rect<Meters>,
    pub ellipse: bool,
    pub reach: Meters,
    pub proximity: f32,
    pub tracks: Vec<ProbeTrack>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeTrack {
    pub channel: u32,
    pub src: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeVoice {
    pub channel: u32,
    pub category: AudioCategory,
    pub src: String,
    pub heard: f32,
}

/// The socket the driver asks on, and the game its questions about the game are read against,
/// once loaded.
struct Probe {
    socket: socket::ProbeSocket<ProbeFrameQuestion>,
    game: Arc<(Mutex<Option<Content>>, Condvar)>,
}

impl Probe {
    fn know(&self, content: &Content) {
        let (game, known) = &*self.game;
        *game.lock().expect("the probe's game") = Some(content.clone());
        known.notify_all();
    }
}

fn loaded(game: &(Mutex<Option<Content>>, Condvar)) -> Content {
    let (content, known) = game;
    let content = known
        .wait_while(content.lock().expect("the probe's game"), |content| {
            content.is_none()
        })
        .expect("the probe's game");
    content.clone().expect("a loaded game")
}

/// The accessibility tree the primary interface last drew, once it is turned on.
#[derive(Resource, Default)]
pub struct DrawnUi(Option<accesskit::TreeUpdate>);

impl DrawnUi {
    pub fn tree(&self) -> Option<&accesskit::TreeUpdate> {
        self.0.as_ref()
    }
}

fn count_input(
    mut seen: ResMut<ProbeInputSeen>,
    mut buttons: MessageReader<MouseButtonInput>,
    mut wheels: MessageReader<MouseWheel>,
    mut keys: MessageReader<KeyboardInput>,
) {
    for button in buttons.read() {
        match button.state {
            ButtonState::Pressed => seen.mouse_presses += 1,
            ButtonState::Released => seen.mouse_releases += 1,
        }
    }
    seen.wheel_turns += wheels.read().count() as u32;
    for key in keys.read() {
        if key.state == ButtonState::Pressed && !key.repeat {
            seen.key_presses += 1;
        }
    }
}

// Players never pay for the accessibility tree; only a client the driver watches turns it on.
fn read_accessibly(gui: Option<Res<GuiContext>>) {
    if let Some(gui) = gui
        && gui.is_added()
    {
        gui.0.enable_accesskit();
    }
}

fn copy_drawn_ui(
    outputs: Query<&EguiOutput, With<PrimaryEguiContext>>,
    mut drawn: ResMut<DrawnUi>,
) {
    if let Some(update) = outputs
        .iter()
        .find_map(|output| output.platform_output.accesskit_update.clone())
    {
        drawn.0 = Some(update);
    }
}

fn count_clicks(gui: Option<Res<GuiContext>>, mut seen: ResMut<ProbeInputSeen>) {
    if let Some(gui) = gui {
        count_click_run(&gui.0, &mut seen);
    }
}

/// Counts the click the interface drawn by `gui` saw released this frame as the run it ends, as
/// egui counts double and triple clicks.
pub fn count_click_run(gui: &egui::Context, seen: &mut ProbeInputSeen) {
    let (run, released) = gui.input(|input| {
        let pointer = &input.pointer;
        let primary = egui::PointerButton::Primary;
        let run = if pointer.button_triple_clicked(primary) {
            Some(3)
        } else if pointer.button_double_clicked(primary) {
            Some(2)
        } else {
            [
                primary,
                egui::PointerButton::Secondary,
                egui::PointerButton::Middle,
            ]
            .into_iter()
            .any(|button| pointer.button_clicked(button))
            .then_some(1)
        };
        (run, pointer.any_released())
    });
    if released {
        seen.click_run = run.unwrap_or(0);
    }
}

fn click_run_open(world: &World) -> bool {
    world
        .get_resource::<GuiContext>()
        .is_some_and(|gui| click_run_still_open(&gui.0))
}

// egui counts a click as the third of a run while the click two before it came within twice the
// double click delay, wherever that click landed.
/// Whether a click now would still count on from the latest one the interface drawn by `gui` saw.
pub fn click_run_still_open(gui: &egui::Context) -> bool {
    let delay = gui.options(|options| options.input_options.max_double_click_delay);
    let since = gui.input(|input| input.pointer.time_since_last_click());
    f64::from(since) < 2.0 * delay
}

// A snapshot scans the whole world, so only a frame the driver is waiting for pays for one.
fn publish(world: &mut World) {
    if !world.contains_resource::<Content>() {
        return;
    }
    if world.is_resource_changed::<Content>() {
        let content = world.resource::<Content>().clone();
        world.non_send::<Probe>().know(&content);
    }
    let asked = world.non_send::<Probe>().socket.asked();
    let answers = world.non_send::<Probe>().socket.answers();
    let mut snapshots = 0;
    for question in asked {
        match question {
            ProbeFrameQuestion::Snapshot => snapshots += 1,
            ProbeFrameQuestion::Frames { since } => {
                let frames = world.resource::<FrameSamples>().since(since);
                answers.give(&serde_json::to_string(&frames).expect("serializable frames"));
            }
            ProbeFrameQuestion::Picture { path } => {
                let answers = answers.clone();
                world.spawn(Screenshot::primary_window()).observe(
                    move |captured: On<ScreenshotCaptured>| {
                        let written = captured
                            .image
                            .clone()
                            .try_into_dynamic()
                            .is_ok_and(|picture| picture.to_rgb8().save(&path).is_ok());
                        answers.give(&written.to_string());
                    },
                );
            }
        }
    }
    if snapshots > 0 {
        let snapshot = snapshot(world);
        let scope = ContentScope::of(world.resource::<Content>());
        // Writing a snapshot out is long work for a frame, and its answer needs nothing more of it.
        AsyncComputeTaskPool::get()
            .spawn(async move {
                let (json, _) = with_content(scope, || {
                    serde_json::to_string(&snapshot).expect("a serializable snapshot")
                });
                for _ in 0..snapshots {
                    answers.give(&json);
                }
            })
            .detach();
    }
}

fn snapshot(world: &mut World) -> ProbeSnapshot {
    let content = world.resource::<Content>().clone();
    let me = session::me(world);
    let area = me
        .and_then(|seen| world.get::<AreaTag>(seen))
        .map(|tag| tag.area);
    let view = view(world);
    let rows = stage::choice_rects(world);
    let input_map = world.resource::<InputMap>().clone();
    let history_tab = history::shown_tab(world);
    let window = world
        .query_filtered::<&Window, With<PrimaryWindow>>()
        .single(world)
        .ok()
        .cloned();
    let system_cursor = world
        .query_filtered::<&CursorOptions, With<PrimaryWindow>>()
        .single(world)
        .is_ok_and(|options| options.visible);
    let sceneries_standing = area::scenery::sceneries_standing(world);
    let on_its_way = render::world::on_its_way(world);
    ProbeSnapshot {
        frame: world.resource::<FrameCount>().0,
        input: ProbeInput {
            cursor: window
                .as_ref()
                .and_then(Window::cursor_position)
                .map(|at| Pos::new(at.x, at.y)),
            system_cursor,
            game_cursor: claims::game_cursor(world),
            click_run_open: click_run_open(world),
            seen: *world.resource::<ProbeInputSeen>(),
        },
        screen: window
            .as_ref()
            .map(|window| Size::new(window.resolution.width(), window.resolution.height())),
        claim: claims::claim(world),
        stage: stage::view(world).map(|view| ProbeStage {
            node: view.node,
            rect: stage::dialogue_rect(world).map(gui::window_rect),
            with: view.with,
            camera: view.camera,
            line: view.line,
            lines: view.lines,
            speaker: view.speaker,
            text: view.text,
            typing: view.typing,
            choices: view
                .choices
                .into_iter()
                .enumerate()
                .map(|(index, choice)| ProbeChoice {
                    label: choice.label,
                    gets: choice.gets,
                    locked: choice.locked,
                    selected: choice.selected,
                    rect: rows.get(&index).copied().map(gui::window_rect),
                })
                .collect(),
        }),
        notifications: notifications(world),
        feed: feed::rows(world),
        feed_place: feed::place(world),
        pops: pop::shown(world),
        history: ProbeHistory {
            open: history_tab.is_some(),
            tab: history_tab.map(|tab| tab.title().to_owned()),
            records: world
                .resource::<HistoryBook>()
                .records()
                .iter()
                .map(|record| record_line(record, &input_map))
                .collect(),
        },
        item_card: item::card::shown(world).map(|(item, rect)| ProbeItemCard {
            item,
            rect: gui::window_rect(rect),
        }),
        skill_card: skill::card::shown(world)
            .zip(hud::pinned_rect::<SkillId>(world))
            .map(|(skill, rect)| ProbeSkillCard {
                skill,
                rect: gui::window_rect(rect),
            }),
        prompt: prompt::tray::view(world).map(|view| ProbePrompt {
            title: view.title.clone(),
            text: view
                .text
                .as_ref()
                .map(|text| text.words(world.resource::<InputMap>())),
            slots: view.slots,
            placed: prompt::tray::placed(world)
                .iter()
                .map(|placed| ProbePlaced {
                    item: placed.stack.item,
                    count: placed.stack.count,
                    short: placed.short,
                })
                .collect(),
            waiting: prompt::tray::waiting(world),
        }),
        crafting: crafting::panel::view(world).map(|_| {
            let (button, ready) = crafting::panel::verdict(world);
            let results = crafting::panel::results(world);
            let stack = |stack: ItemStack| ProbeStack {
                item: stack.item,
                count: stack.count,
            };
            ProbeCrafting {
                recipes: crafting::panel::listed(world)
                    .into_iter()
                    .map(|listed| ProbeRecipe {
                        makes: listed.recipe.makes.iter().copied().map(stack).collect(),
                        from: listed.recipe.from.iter().copied().map(stack).collect(),
                        takes: listed.recipe.takes,
                        chance: listed.recipe.chance,
                        can_make: listed.can_make,
                    })
                    .collect(),
                listing: crafting::panel::listing(world),
                results: results
                    .iter()
                    .map(|result| result.makes.iter().copied().map(stack).collect())
                    .collect(),
                picked: crafting::panel::picked(world).and_then(|picked| {
                    results
                        .iter()
                        .position(|result| result.recipe == picked.recipe)
                }),
                button,
                ready: ready.is_some(),
                progress: crafting::panel::progress(world),
            }
        }),
        shop: shop::counter::view(world).map(|shown| ProbeShop {
            shop: shown.shop,
            offers: shown
                .shop
                .get(&content)
                .sells()
                .zip(&shown.offers)
                .map(|(offer, view)| ProbeOffer {
                    item: offer.item,
                    count: offer.count,
                    left: view.left,
                    refusal: view
                        .refusal
                        .as_ref()
                        .map(|refusal| refusal.describe(&content)),
                })
                .collect(),
            buyback: shown
                .buyback
                .iter()
                .map(|buyback| ProbeStack {
                    item: buyback.sale.item,
                    count: buyback.sale.count,
                })
                .collect(),
        }),
        bag: me
            .and_then(|seen| world.get::<Inventory>(seen))
            .map(|inventory| {
                inventory
                    .slots
                    .iter()
                    .map(|stack| ProbeStack {
                        item: stack.item,
                        count: stack.count,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        quests: me.and_then(|seen| quests(world, seen)).unwrap_or_default(),
        statuses: me
            .and_then(|seen| world.get::<status::Statuses>(seen).cloned())
            .map(|statuses| {
                let now = time::server_now(world);
                statuses
                    .0
                    .iter()
                    .map(|held| ProbeStatus {
                        effect: held.effect,
                        category: held.effect.get(&content).category,
                        left: held.until.zip(now).map(|(until, now)| until.since(now)),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        vfx: vfx_seen(world),
        hotbar: skill::bar::report(world),
        basic_attack: me
            .and_then(|me| world.get::<skill::BasicAttack>(me))
            .and_then(|slot| slot.0),
        skills: me.and_then(|me| skills(world, me)),
        me: me.and_then(|me| body(world, me, view.as_ref())),
        terrain: terrain(world, me, area),
        actors: actors(world, me, area),
        objects: objects(world, area),
        items: items(world, area),
        resources: resources(world, area),
        warps: warps(world, area),
        ui: ui(world),
        covered: covered(world),
        soundscape: soundscape(world),
        weather: area.and_then(|area| weather(world, area, me)),
        transition: world.resource::<ActiveTransition>().phase(),
        daylight: world
            .get_resource::<Daylight>()
            .copied()
            .unwrap_or_default(),
        gpu_memory: gpu_memory(world),
        frame_times: world.resource_mut::<FrameTimes>().0.drain(..).collect(),
        scenery_standing: sceneries_standing > 0 || on_its_way.any(),
        sceneries_standing,
        on_its_way,
        adapter: world
            .get_resource::<RenderAdapterInfo>()
            .map(|adapter| adapter.name.clone()),
        view,
        area,
    }
}

fn gpu_memory(world: &World) -> Bytes {
    let Some(device) = world.get_resource::<RenderDevice>() else {
        return Bytes::default();
    };
    let held = device.wgpu_device().get_internal_counters().hal;
    Bytes(u64::try_from(held.buffer_memory.read() + held.texture_memory.read()).unwrap_or(0))
}

fn record_line(record: &HistoryRecord, inputs: &InputMap) -> ProbeRecord {
    let text = record.entry.text.words(inputs);
    ProbeRecord {
        topic: record.entry.topic,
        by: record.entry.by.clone(),
        text: record
            .entry
            .tally
            .map_or_else(|| text.clone(), |tally| tally.line(&text)),
        mark: record.entry.mark,
    }
}

fn weather(world: &World, area: AreaId, me: Option<Entity>) -> Option<ProbeWeather> {
    let shown = world.get_resource::<ShownWeather>()?;
    let content = world.resource::<Content>();
    let spell = world
        .get_resource::<ShownCourse>()
        .and_then(|course| course.spell(area))
        .and_then(|spell| area::load(content, area).climate.spells.get(spell))
        .map(|spell| spell.name.clone());
    let at = me
        .and_then(|me| world.get::<GlobalTransform>(me))
        .map(GlobalTransform::translation);
    let cell = at.and_then(|at| world.resource::<RainField>().cell_at(at.xz()));
    let sheltered = at.is_some_and(|at| world.resource::<Shelter>().covers(at));
    let open = if sheltered { 0.0 } else { 1.0 };
    Some(ProbeWeather {
        spell,
        clouds: shown.clouds,
        water: shown.water,
        rain: shown.rain,
        raining: cell.map_or(0.0, |cell| cell.rain) * open,
        wetness: cell.map_or(0.0, |cell| cell.wetness) * open,
        sheltered,
        shaded: at
            .zip(world.get_resource::<CloudSheet>())
            .map_or(0.0, |(at, sheet)| {
                sheet.shade_over(at, world.resource::<CloudDrift>().drift())
            }),
        flash: shown.flash,
    })
}

fn soundscape(world: &World) -> ProbeSoundscape {
    let listener = world.resource::<Listener>().0;
    let hearing = world.get_resource::<Hearing>();
    ProbeSoundscape {
        zones: world
            .resource::<soundscape::Soundscape>()
            .0
            .iter()
            .map(|zone| ProbeSoundZone {
                bounds: zone.shape.bounds(),
                ellipse: matches!(zone.shape, SoundscapeShape::Ellipse(_)),
                reach: zone.reach,
                proximity: listener
                    .zip(hearing)
                    .and_then(|(at, hearing)| zone.heard_from(at, hearing))
                    .map_or(0.0, |heard| heard.proximity),
                tracks: zone
                    .channels
                    .iter()
                    .map(|(channel, layer)| ProbeTrack {
                        channel: channel.0,
                        src: layer.src.as_str().to_owned(),
                    })
                    .collect(),
            })
            .collect(),
        voices: world
            .resource::<SoundscapeMixer>()
            .voices()
            .iter()
            .map(|voice| ProbeVoice {
                channel: voice.channel.0,
                category: voice.category,
                src: voice.src.as_str().to_owned(),
                heard: voice.heard(),
            })
            .collect(),
    }
}

fn skills(world: &World, me: Entity) -> Option<ProbeSkills> {
    let content = world.resource::<Content>();
    let class = crate::systems::class::of(world, me)?;
    let learned = world
        .get::<skill::Learned>(me)
        .map(|learned| learned.0.iter().map(rank).collect())
        .unwrap_or_default();
    Some(ProbeSkills {
        class: class.path(content).to_owned(),
        level: crate::systems::class::level(world, me),
        to_spend: crate::systems::class::points_to_spend(world, me).0,
        learned,
        planned: world
            .resource::<skill::window::SkillPlan>()
            .0
            .iter()
            .map(rank)
            .collect(),
        aiming: skill::aim::aiming(world).map(|aimed| aimed.skill),
        reset_prompt: skill::prompt::showing(world),
        view: tree_view(world),
    })
}

fn rank((skill, rank): (&impl ToString, &Rank)) -> ProbeRank {
    ProbeRank {
        skill: skill.to_string(),
        rank: *rank,
    }
}

fn tree_view(world: &World) -> Option<ProbeTreeView> {
    let window = hud::window_rect(world, &skill::window::SkillsWindow)?;
    let view = world.resource::<skill::window::TreeView>();
    Some(ProbeTreeView {
        window: gui::window_rect(window),
        max: hud::window_limit(world, &skill::window::SkillsWindow)
            .map(|max| Size::new(max.x, max.y)),
        viewport: gui::window_rect(view.viewport),
        canvas: Size::new(view.content.x, view.content.y),
        panned: Offset::new(view.offset.x, view.offset.y),
    })
}

fn quests(world: &World, seen: Entity) -> Option<ProbeQuests> {
    let content = world.resource::<Content>();
    let log = world.get::<QuestLog>(seen)?;
    let inventory = world.get::<Inventory>(seen);
    let now = time::LocalClock::of_this_machine().now;
    Some(ProbeQuests {
        active: log
            .active
            .iter()
            .map(|active| ProbeQuest {
                quest: active.quest,
                ready: active.ready(content, world.resource::<Parties>(), inventory),
                left: active.left(now),
                progress: active
                    .quest
                    .get(content)
                    .progress(content, world.resource::<Parties>(), active, inventory)
                    .iter()
                    .map(|progress| [progress.have, progress.need])
                    .collect(),
            })
            .collect(),
        finished: log
            .finished
            .iter()
            .map(|finished| ProbeFinishedQuest {
                quest: finished.quest,
                result: finished.result,
            })
            .collect(),
        tracked: log.tracked.clone(),
    })
}

fn view(world: &mut World) -> Option<ProbeView> {
    let view = render::view_of(world)?;
    let rig = world.resource::<CameraRig>();
    let viewport = view.viewport();
    Some(ProbeView {
        view_proj: view.view_proj(),
        viewport: Rect::from_size(Size::new(viewport.x, viewport.y)),
        yaw: rig.yaw(),
        pitch: rig.pitch(),
        zoom: rig.zoom().step(),
        home: rig.at_home(),
    })
}

/// The ground the probe last sent: the window it covered and how many of the ground's tiles
/// under it were loaded, so it goes out again only once the player has moved on to ground beyond
/// it or more of it answers exactly rather than from the ground's overview.
#[derive(Resource, Default, PartialEq)]
struct TerrainSent(Option<(AreaId, Rect<Meters>, usize)>);

fn terrain(world: &mut World, me: Option<Entity>, area: Option<AreaId>) -> Option<ProbeTerrain> {
    let standing = me.and_then(|me| {
        world
            .get::<RenderPosition>(me)
            .map(|drawn| drawn.0)
            .or_else(|| world.get::<Position>(me).map(|position| position.pos))
    });
    let (Some(area_id), Some(at)) = (area, standing) else {
        world.resource_mut::<TerrainSent>().0 = None;
        return None;
    };
    let reach = area::render::view_reach(
        world.get_resource::<CameraRig>()?.framing(),
        world.get_resource::<VideoPreferences>()?,
    );
    let area = area::load(world.resource::<Content>(), area_id);
    let window = ground_window(at, area.size.bounds(), reach);
    let loaded = area
        .heights
        .size()
        .tiles_within(window)
        .filter(|&tile| area.heights.is_loaded(tile))
        .count();
    let sending = TerrainSent(Some((area_id, window, loaded)));
    let mut sent = world.resource_mut::<TerrainSent>();
    if *sent == sending {
        return None;
    }
    *sent = sending;
    let area = area::load(world.resource::<Content>(), area_id);
    let size = Size::new(window.size.width.floor(), window.size.height.floor());
    let (columns, rows) = (size.width as usize + 1, size.height as usize + 1);
    let heights = (0..rows)
        .flat_map(|row| (0..columns).map(move |column| (column, row)))
        .map(|(column, row)| {
            area.heights
                .height_at(window.origin + Offset::new(column as f32, row as f32))
        })
        .collect();
    Some(ProbeTerrain {
        origin: window.origin,
        size,
        heights,
    })
}

/// The ground around `at` the probe reports: the chunks within the view's reach, within the
/// area, so the window moves only as the player crosses from chunk to chunk.
fn ground_window(at: Pos<Meters>, bounds: Rect<Meters>, reach: Meters) -> Rect<Meters> {
    let low = Chunk::at(Pos::new(at.x - reach.0, at.y - reach.0)).rect();
    let high = Chunk::at(Pos::new(at.x + reach.0, at.y + reach.0)).rect();
    Rect::from_points([low.min(), high.max()])
        .intersection(&bounds)
        .unwrap_or(Rect::new(at, Size::zero()))
}

fn body(world: &World, entity: Entity, view: Option<&ProbeView>) -> Option<ProbeBody> {
    let viewer = session::me(world);
    let marks = viewer
        .and_then(|viewer| world.get::<Attention>(viewer))
        .map(|attention| attention.of(entity).to_vec())
        .unwrap_or_default();
    let locked = commands_locked(world, entity);
    let entity = world.get_entity(entity).ok()?;
    let npc = entity.get::<Npc>().map(|npc| npc.def);
    let at = entity.get::<Position>()?.pos;
    let capsule = entity.get::<Hitbox>()?.0;
    let actor = entity.get::<Actor>()?;
    let drawn = entity.get::<RenderPosition>().map_or(at, |drawn| drawn.0);
    let ground = world
        .get::<AreaTag>(entity.id())
        .map_or(Meters(0.0), |tag| {
            area::load(world.resource::<Content>(), tag.area)
                .heights
                .height_at(drawn)
        });
    let appearance = entity.get::<Appearance>()?.clone();
    let built = ActorModel::get(world.resource::<Content>(), appearance.model);
    let flies = built.airborne();
    let stat = |kind| entity.get::<Stats>().map_or(0.0, |stats| stats.get(kind));
    let stance = held::stance_of(world, entity.id());
    let motion = entity
        .get::<Playing>()
        .filter(|playing| playing.motion.shows_over(actor.action))
        .map(|playing| playing.motion.clone());
    let shown = entity
        .get::<Puppet>()
        .and_then(Puppet::shown)
        .cloned()
        .or_else(|| motion.clone())
        .unwrap_or(Motion::Action(actor.action));
    let pick = entity
        .get::<Puppet>()
        .map_or(0, |puppet| puppet.pick_of(&shown));
    let resolved = built.resolve(stance, &shown, actor.pose, pick);
    let naming = built.naming(stance, &shown, actor.pose);
    let clip = resolved.clip.clone();
    let rate = built.rate(&shown, &resolved, actor.attack_rate, actor.speed);
    let held = entity
        .get::<Puppet>()
        .map(|puppet| {
            puppet
                .held()
                .map(|(hand, hung, prop)| ProbeHeld {
                    hand,
                    item: hung.item(),
                    tool: match hung {
                        Hung::Item(_) => None,
                        Hung::Tool(tool) => Some(tool.prop),
                    },
                    at: world.get::<GlobalTransform>(prop).and_then(|placed| {
                        let at = placed.translation();
                        view.and_then(|view| view.window_point(Pos::new(at.x, at.z), Meters(at.y)))
                    }),
                })
                .collect()
        })
        .unwrap_or_default();
    Some(ProbeBody {
        id: entity.id(),
        engaged: entity.get::<Engaged>().map(|engaged| engaged.0),
        name: entity
            .get::<actor::Name>()
            .map_or_else(String::new, |name| name.name.clone()),
        appearance,
        flies,
        player: entity.contains::<PlayedBy>(),
        at,
        drawn,
        aim: at,
        capsule,
        facing: entity
            .get::<RenderActor>()
            .map_or(actor.facing, |shown| shown.facing),
        action: actor.action,
        pose: actor.pose,
        statuses: carried(world, entity.id()),
        resting: !actor.action.moves(),
        jumps: entity.get::<Position>()?.jumps,
        shown: shown.clone(),
        feet: view.and_then(|view| view.window_point(drawn, ground)),
        head: view.and_then(|view| view.window_point(drawn, ground + capsule.height)),
        health: stat(StatKind::Health),
        max_health: stat(StatKind::MaxHealth),
        npc,
        role: npc
            .and_then(|npc| npc.get(world.resource::<Content>()).nameplate())
            .map(str::to_owned),
        friendly: entity.get::<Attitude>() == Some(&Attitude::Friendly),
        locked,
        marks,
        badges: entity
            .get::<StatusBadges>()
            .map(|badges| badges.0.clone())
            .unwrap_or_default(),
        skeleton: built.skeleton_id(),
        stance,
        clip,
        motion,
        rate,
        naming,
        hands: built.hands().iter().map(|hand| hand.name.clone()).collect(),
        wearing: entity
            .get::<Held>()
            .map(|wearing| {
                wearing
                    .hands
                    .iter()
                    .map(|(&hand, &item)| ProbeWorn { hand, item })
                    .collect()
            })
            .unwrap_or_default(),
        tool: equipment::in_slot(world, entity.id(), &EquipmentPlace::Tool),
        gathering: entity.get::<Working>().map(|working| ProbeGathering {
            resource: working.resource,
            tool: working.tool,
            progress: resource::render::progress(world, working).unwrap_or(0.0),
        }),
        held,
        layered: entity.get::<Layered>().is_some_and(Layered::moving),
    })
}

fn actors(world: &mut World, me: Option<Entity>, area: Option<AreaId>) -> Vec<ProbeBody> {
    let others: Vec<Entity> = world
        .query_filtered::<(Entity, &AreaTag), With<Actor>>()
        .iter(world)
        .filter(|(entity, tag)| Some(*entity) != me && area.is_none_or(|area| tag.area == area))
        .map(|(entity, _)| entity)
        .collect();
    let view = view(world);
    let bodies: Vec<(Entity, ProbeBody)> = others
        .into_iter()
        .filter_map(|entity| body(world, entity, view.as_ref()).map(|body| (entity, body)))
        .collect();
    bodies
        .into_iter()
        .map(|(entity, body)| ProbeBody {
            aim: aim(world, entity, body.at, body.capsule),
            ..body
        })
        .collect()
}

fn objects(world: &mut World, area: Option<AreaId>) -> Vec<ProbeObject> {
    let viewer = session::me(world);
    let attention = viewer
        .and_then(|viewer| world.get::<Attention>(viewer))
        .cloned();
    let objects: Vec<(Entity, ProbeObject)> = world
        .query::<(Entity, &AreaObject, &Position, &AreaTag)>()
        .iter(world)
        .filter(|(.., tag)| area.is_none_or(|area| tag.area == area))
        .filter(|(_, object, ..)| object.look.is_some())
        .map(|(entity, object, at, _)| {
            let hitbox = object.shape.bounds();
            let seen = ProbeObject {
                id: entity,
                name: object.name.clone(),
                title: object.title.clone(),
                at: at.pos,
                aim: hitbox.center(),
                hitbox,
                marks: attention
                    .as_ref()
                    .map(|attention| attention.of(entity).to_vec())
                    .unwrap_or_default(),
                statuses: Vec::new(),
            };
            (entity, seen)
        })
        .collect();
    objects
        .into_iter()
        .map(|(entity, seen)| ProbeObject {
            aim: object_aim(world, entity, seen.hitbox),
            statuses: carried(world, entity),
            ..seen
        })
        .collect()
}

/// The statuses a thing carries, in the order it took them.
fn carried(world: &World, thing: Entity) -> Vec<StatusEffectId> {
    world
        .get::<status::Statuses>(thing)
        .map(|statuses| statuses.0.iter().map(|held| held.effect).collect())
        .unwrap_or_default()
}

/// The ground point to point at to take the body: where the ray from the eye through its capsule
/// lands, at the height nearest its middle where pointing claims the body, else where it stands.
/// A body is given up on at the first height where pointing takes nothing at all: nothing stands
/// in its way there, so it is not for taking.
fn aim(world: &mut World, entity: Entity, at: Pos<Meters>, capsule: Capsule) -> Pos<Meters> {
    const HEIGHTS: [f32; 7] = [0.5, 0.375, 0.625, 0.25, 0.75, 0.125, 0.875];
    let Some(view) = render::view_of(world) else {
        return at;
    };
    let eye = view.eye();
    let foot = ground_height(world, at);
    for share in HEIGHTS {
        let point = world_point(at, foot + Meters(capsule.height.0 * share));
        let Ok(direction) = Dir3::new(point - eye) else {
            continue;
        };
        let ray = Ray3d::new(eye, direction);
        let ground = world
            .get_resource::<GroundHeights>()
            .and_then(|ground| ground.hit(ray));
        match (clicked(world, &Pointed { ray, ground }), ground) {
            (Some(clicked), Some(ground)) if clicked == entity => return ground,
            (Some(_), _) => {}
            (None, _) => break,
        }
    }
    at
}

/// The ground point to point at to take the object: the middle of its shape, or the nearest point
/// of it that pointing at claims the object, else the middle.
fn object_aim(world: &mut World, entity: Entity, hitbox: Rect<Meters>) -> Pos<Meters> {
    const STEPS: u8 = 5;
    let center = hitbox.center();
    let mut spots: Vec<Pos<Meters>> = (0..STEPS)
        .flat_map(|x| (0..STEPS).map(move |y| (x, y)))
        .map(|(x, y)| {
            hitbox.origin
                + Offset::new(
                    hitbox.size.width * (f32::from(x) + 0.5) / f32::from(STEPS),
                    hitbox.size.height * (f32::from(y) + 0.5) / f32::from(STEPS),
                )
        })
        .collect();
    spots.sort_by(|a, b| a.distance_to(center).total_cmp(&b.distance_to(center)));
    std::iter::once(center)
        .chain(spots)
        .find(|&spot| {
            pointed_at(world, spot).is_some_and(|pointed| clicked(world, &pointed) == Some(entity))
        })
        .unwrap_or(center)
}

/// The pointer as it would point at the ground at `spot` from the camera.
fn pointed_at(world: &mut World, spot: Pos<Meters>) -> Option<Pointed> {
    let view = render::view_of(world)?;
    let eye = view.eye();
    let direction = Dir3::new(world_point(spot, ground_height(world, spot)) - eye).ok()?;
    Some(Pointed {
        ray: Ray3d::new(eye, direction),
        ground: Some(spot),
    })
}

fn clicked(world: &mut World, pointed: &Pointed) -> Option<Entity> {
    match claims::claim_at(world, Some(pointed)) {
        ClickClaim::Attack(entity)
        | ClickClaim::Interact(entity)
        | ClickClaim::Aim(entity)
        | ClickClaim::Pickup(entity)
        | ClickClaim::Gather(entity) => Some(entity),
        ClickClaim::Interface
        | ClickClaim::Locked
        | ClickClaim::AimAt(_)
        | ClickClaim::Walk(_)
        | ClickClaim::Nothing => None,
    }
}

fn items(world: &mut World, area: Option<AreaId>) -> Vec<ProbeGroundItem> {
    world
        .query::<(Entity, &DroppedItem, &Position, &AreaTag)>()
        .iter(world)
        .filter(|(.., tag)| area.is_none_or(|area| tag.area == area))
        .map(|(entity, dropped, at, _)| ProbeGroundItem {
            id: entity,
            item: dropped.item,
            count: dropped.count,
            at: at.pos,
        })
        .collect()
}

fn resources(world: &mut World, area: Option<AreaId>) -> Vec<ProbeResource> {
    let content = world.resource::<Content>().clone();
    let standing: Vec<(Entity, ResourceId, u32, Pos<Meters>, Capsule)> = world
        .query::<(Entity, &Resource, &Left, &Position, &Hitbox, &AreaTag)>()
        .iter(world)
        .filter(|(.., tag)| area.is_none_or(|area| tag.area == area))
        .map(|(entity, resource, left, at, hitbox, _)| {
            (entity, resource.def, left.0, at.pos, hitbox.0)
        })
        .collect();
    let working: Vec<(Entity, Entity)> = world
        .query::<(Entity, &Working)>()
        .iter(world)
        .map(|(worker, working)| (worker, working.resource))
        .collect();
    standing
        .into_iter()
        .map(|(entity, def, left, at, capsule)| ProbeResource {
            id: entity,
            resource: def,
            name: def.get(&content).name.clone(),
            at,
            aim: aim(world, entity, at, capsule),
            capsule,
            left,
            room: def.get(&content).max_occupants,
            workers: working
                .iter()
                .filter(|(_, resource)| *resource == entity)
                .map(|(worker, _)| *worker)
                .collect(),
        })
        .collect()
}

fn warps(world: &World, area: Option<AreaId>) -> Vec<ProbeWarp> {
    let Some(area) = area else {
        return Vec::new();
    };
    let content = world.resource::<Content>();
    let scripts = world.resource::<RiftScripts>();
    let mut warps: Vec<ProbeWarp> = area::objects(area::placed(content, area))
        .filter_map(|(index, object)| {
            let owner = OwnerRow::of_object(area, index);
            let warp = scripts
                .may_do(owner)
                .reaches
                .iter()
                .find_map(|reach| match reach {
                    EffectReach::Warp(place) => Some(*place),
                    _ => None,
                })?;
            Some(ProbeWarp {
                name: object.name.clone().unwrap_or_default(),
                to: warp.area,
                at: object.shape.center(),
                rect: object.shape.bounds(),
            })
        })
        .collect();
    warps.sort_by(|a, b| a.name.cmp(&b.name));
    warps
}

fn ui(world: &mut World) -> Vec<ProbeUiElement> {
    let Some(gui) = gui::context(world) else {
        return Vec::new();
    };
    let Some(tree) = world.resource::<DrawnUi>().0.as_ref() else {
        return Vec::new();
    };
    let world_layers = world.resource::<WorldLayers>();
    interface_elements(&gui, tree, |layer| world_layers.contains(layer))
}

/// What the interface drawn by `gui` shows, read from the accessibility `tree` it last drew: text,
/// icons, fields and sliders, less what is scrolled or clipped out of sight. `in_world` tells the
/// layers drawn over the world, such as labels over heads.
pub fn interface_elements(
    gui: &egui::Context,
    tree: &accesskit::TreeUpdate,
    in_world: impl Fn(egui::LayerId) -> bool,
) -> Vec<ProbeUiElement> {
    let drawn: HashMap<u64, egui::WidgetRect> = gui.viewport(|viewport| {
        viewport
            .prev_pass
            .widgets
            .layers()
            .flat_map(|(_, widgets)| widgets.iter())
            .map(|widget| (widget.id.value(), *widget))
            .collect()
    });
    let labels: HashMap<accesskit::NodeId, &str> = tree
        .nodes
        .iter()
        .filter_map(|(id, node)| Some((*id, node.label().or(node.value())?)))
        .collect();
    tree.nodes
        .iter()
        .filter(|(_, node)| shown_role(node.role()))
        .filter_map(|(id, node)| {
            let widget = drawn.get(&id.0)?;
            let bounds = node.bounds()?;
            let rect = egui::Rect::from_min_max(
                egui::pos2(bounds.x0 as f32, bounds.y0 as f32),
                egui::pos2(bounds.x1 as f32, bounds.y1 as f32),
            );
            if rect.width() <= 0.0
                || rect.height() <= 0.0
                || !widget.interact_rect.contains(rect.center())
            {
                return None;
            }
            let role = node.role();
            let image = (role == accesskit::Role::Image)
                .then(|| node.label().map(str::to_owned))
                .flatten();
            let text = (role != accesskit::Role::Image)
                .then(|| {
                    node.label()
                        .or(node.value())
                        .or_else(|| {
                            node.labelled_by()
                                .iter()
                                .find_map(|by| labels.get(by).copied())
                        })
                        .map(|text| text.to_string())
                })
                .flatten()
                .filter(|text| !text.is_empty());
            let slider = (role == accesskit::Role::Slider)
                .then(|| node.numeric_value())
                .flatten()
                .map(|value| value as f32);
            let checked = node
                .toggled()
                .map(|toggled| toggled == accesskit::Toggled::True);
            let labelled_by = node
                .labelled_by()
                .iter()
                .find_map(|by| labels.get(by))
                .map(|label| label.to_string());
            let editable = role == accesskit::Role::TextInput;
            if text.is_none() && image.is_none() && !editable && slider.is_none() {
                return None;
            }
            Some(ProbeUiElement {
                text,
                image,
                editable,
                button: role == accesskit::Role::Button,
                focused: tree.focus == *id,
                click_through: float::layer_at(gui, rect.center()) != Some(widget.layer_id),
                in_world: in_world(widget.layer_id),
                slider,
                checked,
                labelled_by,
                rect: gui::window_rect(rect),
            })
        })
        .collect()
}

fn shown_role(role: accesskit::Role) -> bool {
    use accesskit::Role;
    matches!(
        role,
        Role::Label
            | Role::Button
            | Role::CheckBox
            | Role::RadioButton
            | Role::ComboBox
            | Role::Slider
            | Role::TextInput
            | Role::Image
            | Role::Link
            | Role::Splitter
    )
}

fn notifications(world: &mut World) -> ProbeNotifications {
    let bubbles = bubble::shown(world);
    let edge_folds = bubble::edge_folds(world);
    ProbeNotifications {
        bubbles,
        edge_folds,
        captions: notification::captions(world),
        error: error::shown(world),
        milestones: notification::milestones(world),
        intro: intro::shown(world),
    }
}

fn covered(world: &mut World) -> Vec<Rect<WindowPx>> {
    gui::context(world).map_or_else(Vec::new, |gui| interface_covers(&gui))
}

/// Where the interface drawn by `gui` takes the pointer: each rect edges included, as egui
/// hit-tests.
pub fn interface_covers(gui: &egui::Context) -> Vec<Rect<WindowPx>> {
    let layers: Vec<egui::LayerId> =
        gui.memory(|memory| memory.areas().visible_layer_ids().into_iter().collect());
    // egui hands a click to a widget this close to it as well as one under it.
    let reach = gui.global_style().interaction.interact_radius;
    layers
        .into_iter()
        .filter(|&layer| layer != egui::LayerId::background())
        .filter_map(|layer| egui::AreaState::load(gui, layer.id))
        .filter(|area| area.interactable)
        .map(|area| area.rect().expand(reach))
        .filter(|rect| rect.width() > 0.0 && rect.height() > 0.0)
        .map(gui::window_rect)
        .collect()
}

const VFX_MEMORY: Seconds = Seconds(3.0);
#[derive(Resource, Default)]
struct VfxSeen(Vec<SeenVfx>);

/// A vfx seen starting, and for one that plays until it is removed, when it was found gone.
struct SeenVfx {
    entity: Entity,
    instance: VfxInstance,
    gone: Option<time::UnixMillis>,
}

impl SeenVfx {
    fn over(&self) -> Option<time::UnixMillis> {
        self.instance.until.or(self.gone)
    }
}

fn remember_vfx(
    add: On<Add<VfxInstance>>,
    instances: Query<&VfxInstance>,
    mut seen: ResMut<VfxSeen>,
) {
    if let Ok(instance) = instances.get(add.entity) {
        seen.0.push(SeenVfx {
            entity: add.entity,
            instance: instance.clone(),
            gone: None,
        });
    }
}

fn vfx_seen(world: &mut World) -> Vec<ProbeVfx> {
    let now = time::server_now(world);
    let gone: Vec<bool> = world
        .resource::<VfxSeen>()
        .0
        .iter()
        .map(|seen| world.get::<VfxInstance>(seen.entity).is_none())
        .collect();
    let mut seen = world.resource_mut::<VfxSeen>();
    if let Some(now) = now {
        for (seen, gone) in seen.0.iter_mut().zip(gone) {
            if gone && seen.gone.is_none() {
                seen.gone = Some(now);
            }
        }
        seen.0
            .retain(|seen| seen.over().is_none_or(|over| now < over.after(VFX_MEMORY)));
    }
    seen.0
        .iter()
        .map(|seen| ProbeVfx {
            vfx: seen.instance.vfx,
            site: seen.instance.site.clone(),
            over: now.zip(seen.over()).is_some_and(|(now, over)| over <= now),
        })
        .collect()
}
