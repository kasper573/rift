mod editor;
mod frames;
mod game;
mod hitches;
mod sight;
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
    GameAnswers, ProbeArea, ProbeBinding, ProbeBindings, ProbeButton, ProbeFrameQuestion,
    ProbeGame, ProbeGameQuestion, ProbeGesture, ProbeInterfaceIcon, ProbeItem, ProbeNamedObject,
    ProbeNpc, ProbeProp, ProbeResident, ProbeSkill, ProbeSpawner, ProbeWater,
};
pub use hitches::{SystemTimes, time_systems, timing_layer};
pub use sight::{
    ProbeAnswer, ProbeBody, ProbeChatLine, ProbeConversation, ProbeCrafting, ProbeFinishedQuest,
    ProbeGathering, ProbeGroundItem, ProbeHand, ProbeLetter, ProbeObject, ProbePrompt, ProbeQuest,
    ProbeQuests, ProbeRank, ProbeRecipe, ProbeRecord, ProbeRequest, ProbeResource, ProbeSaid,
    ProbeShop, ProbeSight, ProbeSkills, ProbeStack, ProbeStatus, ProbeWare, ProbeWarp, ProbeWorn,
    sight,
};

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use ui::egui;
use ui::float;

use bevy::diagnostic::FrameCount;
use bevy::ecs::entity::{EntityMapper, MapEntities};
use bevy::input::ButtonState;
use bevy::input::keyboard::KeyboardInput;
use bevy::input::mouse::{MouseButtonInput, MouseWheel};
use bevy::prelude::*;
use bevy::render::renderer::{RenderAdapterInfo, RenderDevice};
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::tasks::AsyncComputeTaskPool;
use bevy::window::{CursorOptions, PrimaryWindow};
use bevy_egui::{EguiOutput, EguiPostUpdateSet, PrimaryEguiContext};
use bevy_replication::ServerEntities;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::audio::Hearing;
use crate::core::audio::mix::AudioCategory;
use crate::core::audio::playback::{self, Listener};
use crate::core::audio::soundscape::{self, SoundscapeMixer, SoundscapeShape};
use crate::core::card::{CardMaterial, CardStyleDef, CardStyleId};
use crate::core::content::{Content, ContentScope, with_content};
use crate::core::geometry::Capsule;
use crate::core::gui::{self, GuiContext, WorldLayers};
use crate::core::math::{
    MatrixSchema, Offset, OffsetSchema, Pos, PosSchema, Rect, RectSchema, Rgb, Size, SizeSchema,
};
use crate::core::render::sample::{
    self, FrameSamplingPlugin, FrameWork, GpuReportsRead, RenderReports,
};
use crate::core::render::transition::{ActiveTransition, ScreenTransitionPhase};
use crate::core::render::world::clouds::{CloudDrift, CloudSheet};
use crate::core::render::world::haze::Haze;
use crate::core::render::world::layers::Layered;
use crate::core::render::world::outline::Outline;
use crate::core::render::world::sky::{Daylight, SkyLights};
use crate::core::render::world::{OnItsWay, missing_drawn};
use crate::core::render::{
    self, CameraRig, Look, Pointed, View, ground_height, surface_height, world_point,
};
use crate::core::script::DialogueCamera;
use crate::core::stage::card::HungCardLamp;
use crate::core::time;
use crate::core::units::{
    Bytes, Degrees, Meters, Millis, PlaybackRate, Radians, Seconds, WindowPx,
};
use crate::core::vfx::standing::StandingVfx;
use crate::core::vfx::{VfxId, VfxInstance, VfxSite};
use crate::systems::actor::held::{self};
use crate::systems::actor::render::{Hung, Puppet, RenderActor};
use crate::systems::actor::skeleton::{Clip, Naming};
use crate::systems::actor::stance::HandIndex;
use crate::systems::actor::{self, Actor, ActorModel, Aloft, Appearance, Hitbox, Motion, Playing};
use crate::systems::area::rain::{RainField, Shelter};
use crate::systems::area::shown_weather::ShownWeather;
use crate::systems::area::{self, AreaId, AreaObject, AreaTag, PropId};
use crate::systems::area_instance;
use crate::systems::area_instance::entrance::ShownEntrancePanel;
use crate::systems::area_map;
use crate::systems::attention::AttentionId;
use crate::systems::crafting::{self};
use crate::systems::dialogue::stage;
use crate::systems::history::widget as history;
use crate::systems::hud;
use crate::systems::hud::menu::MenuTabName;
use crate::systems::input::claims::{self, ClickClaim};
use crate::systems::input::scope::InputScope;
use crate::systems::interface::CursorId;
use crate::systems::item::bag_view;
use crate::systems::item::{self, ItemCategory, ItemId};
use crate::systems::mail::{self, page::ShownMailPage};
use crate::systems::market::{self, window::ShownMarketWindow};
use crate::systems::messages;
use crate::systems::movement::{Position, RenderPosition};
use crate::systems::notification::{self, NotificationRows, bubble, error, feed, intro, pop};
use crate::systems::player::session;
use crate::systems::prompt;
use crate::systems::quest::{self, QuestId};
use crate::systems::save::{self, notice::ShownSaving};
use crate::systems::settings::Placed;
use crate::systems::shop::{self};
use crate::systems::skill::{self, SkillId, aim::SpotAim, bar::SlotReport};
use crate::systems::social_request::{self, card::ShownRequestCard};
use crate::systems::terminal::{self, chat::ShownChat};

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
    pub shown: Vec<Option<ProbeShown>>,
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
        let mut answers = GameAnswers::default();
        move |line| {
            let content = loaded(&game);
            match read_question(&content, line) {
                Ok(question) => answers.answer(&content, &question),
                Err(error) => {
                    bevy::log::error!("the probe's driver asked {line}: {error}");
                    std::process::exit(1);
                }
            }
        }
    });
    app.insert_non_send(Probe { socket, game })
        .add_plugins((ProbeFramesPlugin, ProbeInterfacePlugin))
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

/// One frame of the client: what the client knows of the game, its sight, and how its screen
/// shows it. Content ids (de)serialize by key, so a driver reads a snapshot within a content scope.
/// World positions are metres on the ground, which `view` places in the window; window positions
/// are the window's logical pixels.
#[derive(MapEntities, Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeSnapshot {
    #[serde(flatten)]
    #[entities]
    pub sight: ProbeSight,
    pub frame: u32,
    pub input: ProbeInput,
    /// Where the player is, which decides the actions that are read and what stepping out does.
    pub scope: InputScope,
    #[entities]
    pub claim: ClickClaim,
    #[schemars(with = "Option<SizeSchema>")]
    pub screen: Option<Size<WindowPx>>,
    pub view: Option<ProbeView>,
    /// How each body of the area shows on screen, the player's own among them.
    #[entities]
    pub drawn: Vec<ProbeDrawnBody>,
    /// How each item on the ground of the area shows on screen.
    #[entities]
    pub drawn_items: Vec<ProbeDrawnItem>,
    /// What the interface shows: text, icons, fields and sliders, less what is scrolled or clipped
    /// out of sight.
    pub ui: Vec<ProbeUiElement>,
    /// Where the interface takes the pointer: each rect edges included, as egui hit-tests.
    #[schemars(with = "Vec<RectSchema>")]
    pub covered: Vec<Rect<WindowPx>>,
    /// The conversation as the dialogue box shows it.
    #[entities]
    pub stage: Option<ProbeStage>,
    pub notifications: ProbeNotifications,
    pub feed: Vec<String>,
    pub feed_place: feed::FeedPlace,
    /// What the interface shows of saving.
    pub saving: ShownSaving,
    /// The request cards on the HUD, the one in front first.
    pub social_requests: Vec<ShownRequestCard>,
    /// The letter the Mail page shows, and the one being written.
    pub mail: ShownMailPage,
    /// The tabs of chat, which is shown and how much waits on each.
    pub chat: ShownChat,
    /// The townsfolk the minimap and the map mark this frame, with the marks they wear.
    #[entities]
    pub map_marks: Vec<ProbeMapMark>,
    /// The rings of marks open round a townsperson on the minimap or the map.
    #[entities]
    pub map_rings: Vec<ProbeMapRing>,
    /// How many of the squares of ground the maps show have their picture yet.
    pub map_tiles: ProbeMapTiles,
    /// The bag as each window that shows it shows it.
    pub bag_views: Vec<ProbeBagView>,
    /// What is carried over a bag, while something is.
    pub bag_carried: Option<ProbeBagCarried>,
    /// The entrance panel, while the player chooses how to go into an instance.
    pub entrance_panel: Option<ShownEntrancePanel>,
    /// The options open for another player: what the player can do with them.
    pub actor_options: Option<crate::systems::actor::options::ShownActorOptions>,
    /// The market's window, while the market is open.
    pub market_window: Option<ShownMarketWindow>,
    /// While a spot is aimed at: what asks for it, where the pointer's spot is and what the
    /// spot has against it.
    pub spot_aim: Option<SpotAim>,
    pub pops: Vec<pop::ShownPop>,
    pub history_page: ProbeHistoryPage,
    /// What the player has placed on the shop's counter to sell, while the shop is open.
    pub shop_counter: Option<ProbeShopCounter>,
    /// What the player has placed in the prompt's tray, while a prompt is open.
    pub prompt_tray: Option<ProbePromptTray>,
    /// What the crafting panel makes of what is placed, while it is open.
    pub crafting_panel: Option<ProbeCraftingPanel>,
    pub item_card: Option<ProbeItemCard>,
    pub skill_card: Option<ProbeSkillCard>,
    pub quest_card: Option<ProbeQuestCard>,
    pub bar_slots: Vec<SlotReport>,
    /// What the skill tree shows beside the skills learned.
    pub skill_tree: ProbeSkillTree,
    pub vfx: Vec<ProbeVfx>,
    pub soundscape: ProbeSoundscape,
    /// The weather as the client draws it, where the player stands.
    pub drawn_weather: Option<ProbeWeather>,
    pub transition: ScreenTransitionPhase,
    pub daylight: Daylight,
    /// Where the sun and the moon stand over the shown area, which has a sky of its own.
    pub heavens: Option<ProbeHeavens>,
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

/// How a body shows on screen.
#[derive(MapEntities, Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeDrawnBody {
    #[entities]
    #[schemars(with = "u64")]
    pub id: Entity,
    /// Where it shows, which runs behind where it stands for others and ahead of it for the
    /// player's own walk.
    #[schemars(with = "PosSchema")]
    pub drawn: Pos<Meters>,
    /// The way its body is turned on screen.
    pub facing: Radians,
    /// The motion its body shows: what it does, or a move, a gesture, a flinch at a blow that
    /// just landed on it.
    pub shown: Motion,
    /// The clip it plays for the motion it shows, as its stance resolves it.
    pub clip: Clip,
    /// How fast that clip plays: a hit at its attack speed, a gait as fast as covers its speed.
    pub rate: PlaybackRate,
    /// Where its skeleton's sets named that clip.
    pub naming: Naming,
    /// Where its feet and the top of its head show in the window, as drawn: nowhere, behind the
    /// camera.
    #[schemars(with = "Option<PosSchema>")]
    pub feet: Option<Pos<WindowPx>>,
    #[schemars(with = "Option<PosSchema>")]
    pub head: Option<Pos<WindowPx>>,
    /// The props its rig holds as drawn, each where it shows in the window.
    pub held: Vec<ProbeHeld>,
    /// Whether any layer moves its joints this frame.
    pub layered: bool,
}

/// How an item on the ground shows on screen.
#[derive(MapEntities, Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeDrawnItem {
    #[entities]
    #[schemars(with = "u64")]
    pub id: Entity,
    /// Where the plate that names it is drawn, when one is.
    #[schemars(with = "Option<RectSchema>")]
    pub plate: Option<Rect<WindowPx>>,
    /// Whether the vfx of its rarity plays on it.
    pub effects: bool,
    /// Whether a line is drawn round it.
    pub outlined: bool,
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
    /// Whether a checkbox is ticked, or a tab or a row is the one selected; none for anything
    /// else.
    pub checked: Option<bool>,
    /// The text of the label that names the element, as a form's label names the field beside it.
    pub labelled_by: Option<String>,
    #[schemars(with = "RectSchema")]
    pub rect: Rect<WindowPx>,
}

#[derive(MapEntities, Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeStage {
    pub node: String,
    /// The dialogue box as last drawn.
    #[schemars(with = "Option<RectSchema>")]
    pub rect: Option<Rect<WindowPx>>,
    #[entities]
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

/// A stack placed in a prompt's tray.
/// The history page, while the menu shows it, and the tab it shows.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeHistoryPage {
    pub open: bool,
    pub tab: Option<String>,
}

/// What the player has placed to sell, stack by stack, and what the shop pays for it all.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeShopCounter {
    pub to_sell: Vec<ProbeStack>,
    pub pays: Vec<ProbeStack>,
}

/// What the player has placed in a prompt's tray.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbePromptTray {
    pub placed: Vec<ProbePlaced>,
    /// Whether what is placed was submitted and the server has yet to answer.
    pub waiting: bool,
}

/// What the crafting panel makes of the items placed, and what its button says.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeCraftingPanel {
    /// What the items placed make, one result for each recipe they are a whole number of times:
    /// everything that recipe would give for them.
    pub results: Vec<Vec<ProbeStack>>,
    /// Which of the results its button would make: the only one, or of several the one chosen.
    pub picked: Option<usize>,
    pub button: String,
    /// Whether the button can be pressed.
    pub ready: bool,
}

/// What the skill tree shows beside the skills learned: the ranks planned, the skill being
/// aimed, whether the offer to unlearn is up, and the pane, while the menu shows it.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ProbeSkillTree {
    pub planned: Vec<ProbeRank>,
    pub aiming: Option<SkillId>,
    pub reset_prompt: bool,
    pub view: Option<ProbeTreeView>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbePlaced {
    pub item: ItemId,
    pub count: u32,
    /// Whether the bag no longer covers what was placed.
    pub short: bool,
}

/// A townsperson's mark on a map: which map, where on screen, and the marks they wear, most
/// pressing first. The map draws the first, with a sign when there are more.
#[derive(MapEntities, Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeMapMark {
    /// `minimap` or `map`.
    pub view: String,
    #[entities]
    #[schemars(with = "u64")]
    pub actor: Entity,
    #[schemars(with = "PosSchema")]
    pub at: Pos<WindowPx>,
    pub marks: Vec<AttentionId>,
}

/// The marks of a townsperson laid round their spot on a map, and how far the ring is open,
/// from 0 to 1.
#[derive(MapEntities, Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeMapRing {
    /// `minimap` or `map`.
    pub view: String,
    #[entities]
    #[schemars(with = "u64")]
    pub actor: Entity,
    pub marks: Vec<AttentionId>,
    pub open: f32,
}

/// The squares of ground the maps show: how many, and how many of them show their picture yet.
/// `ahead` counts the squares asked for before a map shows them (the whole area, and the full
/// map's zoom steps round the player) that have no picture yet.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, Default)]
pub struct ProbeMapTiles {
    pub wanted: u32,
    pub shown: u32,
    pub ahead: u32,
}

/// One window's bag: the words and the type of item it is narrowed to, and the slots it shows,
/// in the order shown. A bag nothing narrows shows every slot, empty or not.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeBagView {
    pub placement: Placed,
    pub words: String,
    pub category: Option<ItemCategory>,
    pub shown: Vec<u32>,
}

/// Something carried over a bag: the slot it was taken from when it is a stack of the bag, the
/// slot under the pointer, and what letting go there does, as the hint says it.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeBagCarried {
    pub from: Option<u32>,
    pub over: Option<u32>,
    pub does: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeItemCard {
    pub item: ItemId,
    #[schemars(with = "RectSchema")]
    pub rect: Rect<WindowPx>,
    pub tilt: ProbeCardTilt,
    pub lamp: Option<ProbeCardLamp>,
    pub material: CardMaterial,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeSkillCard {
    pub skill: SkillId,
    #[schemars(with = "RectSchema")]
    pub rect: Rect<WindowPx>,
    pub tilt: ProbeCardTilt,
    pub lamp: Option<ProbeCardLamp>,
    pub material: CardMaterial,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeQuestCard {
    pub quest: QuestId,
    #[schemars(with = "RectSchema")]
    pub rect: Rect<WindowPx>,
    pub tilt: ProbeCardTilt,
    pub lamp: Option<ProbeCardLamp>,
    pub material: CardMaterial,
}

/// How far a card is turned toward the pointer: `aside` about its upright axis and `above` about
/// its level one, so the part under the pointer sinks back.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug)]
pub struct ProbeCardTilt {
    pub aside: Degrees,
    pub above: Degrees,
}

/// The lamp a card is lit by, once it is: where over the window it hangs, as shares of the
/// window's width and height from its top left corner, how far in front of the window, in window
/// heights, its colour, and how much of the card's light is its.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug)]
pub struct ProbeCardLamp {
    pub across: f32,
    pub down: f32,
    pub height: f32,
    pub color: Rgb,
    pub strength: f32,
}

impl From<HungCardLamp> for ProbeCardLamp {
    fn from(lamp: HungCardLamp) -> ProbeCardLamp {
        ProbeCardLamp {
            across: lamp.across,
            down: lamp.down,
            height: lamp.height,
            color: Rgb::nearest(lamp.color),
            strength: lamp.strength,
        }
    }
}

impl From<ui::card::CardTilt> for ProbeCardTilt {
    fn from(tilt: ui::card::CardTilt) -> ProbeCardTilt {
        ProbeCardTilt {
            aside: Degrees(tilt.aside),
            above: Degrees(tilt.above),
        }
    }
}

/// A vfx seen starting lately, with the site it plays on; `over` once its time has run out. One
/// shorter than the gap between two snapshots is still reported in the next.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ProbeVfx {
    pub vfx: VfxId,
    pub site: VfxSite,
    pub over: bool,
}

/// How the menu shows the skill tree: the canvas (backdrop and nodes) pans within the viewport,
/// which the canvas always fills.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ProbeTreeView {
    #[schemars(with = "RectSchema")]
    pub viewport: Rect<WindowPx>,
    #[schemars(with = "SizeSchema")]
    pub canvas: Size<WindowPx>,
    #[schemars(with = "OffsetSchema")]
    pub panned: Offset<WindowPx>,
}

/// The weather over the shown area as the client draws it: how much cloud covers the sky, how
/// much water it holds and how hard it rains under the wettest of it, each 0 to 1; and where the
/// player stands, how hard it rains, how wet the ground lies, whether they stand sheltered from the rain, and how much of the
/// sun's or the moon's light the cloud overhead takes from them, 0 none to 1 all of it; how
/// brightly lightning lights the world this moment; and how far the fog the camera's eye stands
/// in lets it see, looking level, or null where no fog bank lies over it.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeWeather {
    pub clouds: f32,
    pub water: f32,
    pub rain: f32,
    pub raining: f32,
    pub wetness: f32,
    pub sheltered: bool,
    pub shaded: f32,
    pub flash: f32,
    pub sight: Option<Meters>,
}

/// How high the sun and the moon stand over the horizon as the client draws them, under it
/// below zero; no moon in an area whose light stands fixed.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq)]
pub struct ProbeHeavens {
    pub sun: Degrees,
    pub moon: Option<Degrees>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ProbeSoundscape {
    pub zones: Vec<ProbeSoundZone>,
    pub voices: Vec<ProbeVoice>,
    /// Every sound effect that sounds right now, by its file, once for each time it plays.
    pub effects: Vec<String>,
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
                let answers = answers.clone();
                AsyncComputeTaskPool::get()
                    .spawn(async move {
                        answers.give(&serde_json::to_string(&frames).expect("serializable frames"));
                    })
                    .detach();
            }
            ProbeFrameQuestion::Aim { of } => {
                let of = world.resource::<ServerEntities>().client_of(of);
                let aim = of.and_then(|of| aim_at(world, of));
                answers.give(&serde_json::to_string(&aim).expect("a serializable aim"));
            }
            ProbeFrameQuestion::Show { spots } => {
                let shown = show(world, &spots);
                answers.give(&serde_json::to_string(&shown).expect("serializable spots"));
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
        let mut snapshot = snapshot(world);
        snapshot.map_entities(&mut ServerNames(world.resource::<ServerEntities>()));
        let scope = ContentScope::of(world.resource::<Content>());
        // Writing a snapshot out is long work for a frame, and needs nothing more of it.
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

/// Where a spot shows in the window: the whole pixel the pointer rests on to click it, where a
/// click there lands, and how much ground the pixel spans, the nearest a click can come to any
/// spot there.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq)]
pub struct ProbeShown {
    #[schemars(with = "PosSchema")]
    pub at: Pos<WindowPx>,
    #[schemars(with = "PosSchema")]
    pub lands: Pos<Meters>,
    pub span: Meters,
}

/// Names the client's entities as the server knows them, so an id a snapshot tells of means the
/// same to every client of the server. What the server never showed is no entity there.
struct ServerNames<'a>(&'a ServerEntities);

impl EntityMapper for ServerNames<'_> {
    fn get_mapped(&mut self, source: Entity) -> Entity {
        self.0.server_of(source).unwrap_or(Entity::PLACEHOLDER)
    }

    fn set_mapped(&mut self, _source: Entity, _target: Entity) {}
}

/// What the client shows this frame, but for the ground around the player, which is still to be
/// sampled where it is due.
fn snapshot(world: &mut World) -> ProbeSnapshot {
    let content = world.resource::<Content>().clone();
    let sight = sight::sight_as_held(world);
    let me = session::me(world);
    let area = sight.area;
    let view = view(world);
    let rows = stage::choice_rects(world);
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
        scope: *world.resource::<InputScope>(),
        screen: window
            .as_ref()
            .map(|window| Size::new(window.resolution.width(), window.resolution.height())),
        claim: claims::claim(world),
        drawn: drawn_bodies(world, area, view.as_ref()),
        drawn_items: drawn_items(world, &sight.items),
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
        saving: save::notice::shown(world),
        social_requests: social_request::card::shown(world),
        mail: mail::page::shown(world),
        chat: terminal::chat::shown(world),
        map_marks: map_marks(world),
        map_rings: map_rings(world),
        map_tiles: map_tiles(world),
        bag_views: bag_views(world),
        bag_carried: bag_carried(world),
        entrance_panel: area_instance::entrance::shown_panel(world),
        actor_options: crate::systems::actor::options::shown(world),
        market_window: market::window::shown(world),
        spot_aim: skill::aim::spot_aim(world),
        pops: pop::shown(world),
        history_page: ProbeHistoryPage {
            open: history_tab.is_some(),
            tab: history_tab.map(|tab| tab.named(&messages::of(&content).history).to_owned()),
        },
        item_card: item::card::shown(world).map(|(item, rect)| ProbeItemCard {
            item,
            rect: gui::window_rect(rect),
            tilt: item::card::tilt(world).into(),
            lamp: item::card::lamp(world).map(Into::into),
            material: card_material(&content, item.get(&content).rarity.get(&content).card_style),
        }),
        skill_card: skill::card::shown(world)
            .zip(hud::pinned_rect::<SkillId>(world))
            .map(|(skill, rect)| ProbeSkillCard {
                skill,
                rect: gui::window_rect(rect),
                tilt: skill::card::tilt(world).into(),
                lamp: skill::card::lamp(world).map(Into::into),
                material: card_material(&content, skill.get(&content).card_style),
            }),
        quest_card: quest::card::shown(world).map(|(quest, rect)| ProbeQuestCard {
            quest,
            rect: gui::window_rect(rect),
            tilt: quest::card::tilt(world).into(),
            lamp: quest::card::lamp(world).map(Into::into),
            material: card_material(&content, quest.get(&content).card_style),
        }),
        prompt_tray: prompt::view(world).map(|_| ProbePromptTray {
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
        crafting_panel: crafting::view(world).map(|_| {
            let (button, ready) = crafting::panel::verdict(world);
            let results = crafting::panel::results(world);
            ProbeCraftingPanel {
                results: results
                    .iter()
                    .map(|result| result.makes.iter().map(sight::stack).collect())
                    .collect(),
                picked: crafting::panel::picked(world).and_then(|picked| {
                    results
                        .iter()
                        .position(|result| result.recipe == picked.recipe)
                }),
                button,
                ready: ready.is_some() || crafting::progress(world).is_some(),
            }
        }),
        shop_counter: shop::counter::placed_sale(world).map(|sale| ProbeShopCounter {
            to_sell: sale.stacks.iter().map(sight::stack).collect(),
            pays: sale.pays.iter().map(sight::stack).collect(),
        }),
        vfx: vfx_seen(world),
        bar_slots: skill::bar::report(world),
        skill_tree: ProbeSkillTree {
            planned: world
                .resource::<skill::window::SkillPlan>()
                .0
                .iter()
                .map(sight::rank)
                .collect(),
            aiming: skill::aim::aiming(world).and_then(skill::aim::Aimed::skill),
            reset_prompt: skill::prompt::showing(world),
            view: tree_view(world),
        },
        ui: ui(world),
        covered: covered(world),
        soundscape: soundscape(world),
        drawn_weather: area.and(weather(world, me)),
        transition: world.resource::<ActiveTransition>().phase(),
        daylight: world
            .get_resource::<Daylight>()
            .copied()
            .unwrap_or_default(),
        heavens: area.and_then(|area| heavens(world, area)),
        gpu_memory: gpu_memory(world),
        frame_times: world.resource_mut::<FrameTimes>().0.drain(..).collect(),
        scenery_standing: sceneries_standing > 0 || on_its_way.any(),
        sceneries_standing,
        on_its_way,
        adapter: world
            .get_resource::<RenderAdapterInfo>()
            .map(|adapter| adapter.name.clone()),
        view,
        sight,
    }
}

fn gpu_memory(world: &World) -> Bytes {
    let Some(device) = world.get_resource::<RenderDevice>() else {
        return Bytes::default();
    };
    let held = device.wgpu_device().get_internal_counters().hal;
    Bytes(u64::try_from(held.buffer_memory.read() + held.texture_memory.read()).unwrap_or(0))
}

fn weather(world: &World, me: Option<Entity>) -> Option<ProbeWeather> {
    let shown = world.get_resource::<ShownWeather>()?;
    let at = me
        .and_then(|me| world.get::<GlobalTransform>(me))
        .map(GlobalTransform::translation);
    let cell = at.and_then(|at| world.resource::<RainField>().cell_at(at.xz()));
    let sheltered = at.is_some_and(|at| world.resource::<Shelter>().covers(at));
    let open = if sheltered { 0.0 } else { 1.0 };
    Some(ProbeWeather {
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
        sight: world
            .get_resource::<CameraRig>()
            .zip(world.try_query::<&Haze>())
            .and_then(|(rig, mut hazes)| hazes.iter(world).next()?.sight_at(rig.eye())),
    })
}

fn heavens(world: &World, area: AreaId) -> Option<ProbeHeavens> {
    let noon = area::load(world.resource::<Content>(), area).lighting.sun?;
    let daylight = world.get_resource::<Daylight>().copied()?;
    let lights = SkyLights::at(
        daylight,
        noon.elevation,
        noon.azimuth,
        noon.color.color(),
        0.0,
        0.0,
        world.get_resource::<Look>()?,
    );
    Some(ProbeHeavens {
        sun: lights.sun.elevation,
        moon: lights.moon.map(|moon| moon.elevation),
    })
}

fn soundscape(world: &World) -> ProbeSoundscape {
    let content = world.resource::<Content>();
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
                    .layers
                    .iter()
                    .map(|layer| ProbeTrack {
                        channel: layer.channel.0,
                        src: layer.src.as_str().to_owned(),
                    })
                    .collect(),
            })
            .collect(),
        effects: playback::sounding(world)
            .into_iter()
            .map(|sfx| sfx.path(content).to_owned())
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

fn tree_view(world: &World) -> Option<ProbeTreeView> {
    if hud::menu::showing(world) != Some(MenuTabName::Skills) {
        return None;
    }
    let view = world.resource::<skill::window::TreeView>();
    Some(ProbeTreeView {
        viewport: gui::window_rect(view.viewport),
        canvas: Size::new(view.content.x, view.content.y),
        panned: Offset::new(view.offset.x, view.offset.y),
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

fn drawn_bodies(
    world: &mut World,
    area: Option<AreaId>,
    view: Option<&ProbeView>,
) -> Vec<ProbeDrawnBody> {
    let bodies: Vec<Entity> = world
        .query_filtered::<(Entity, &AreaTag), With<Actor>>()
        .iter(world)
        .filter(|(_, tag)| area.is_none_or(|area| tag.area == area))
        .map(|(entity, _)| entity)
        .collect();
    bodies
        .into_iter()
        .filter_map(|entity| drawn_body(world, entity, view))
        .collect()
}

fn drawn_body(world: &World, entity: Entity, view: Option<&ProbeView>) -> Option<ProbeDrawnBody> {
    let entity = world.get_entity(entity).ok()?;
    let at = entity.get::<Position>()?.pos;
    let capsule = entity.get::<Hitbox>()?.0;
    let actor = entity.get::<Actor>()?;
    let drawn = entity.get::<RenderPosition>().map_or(at, |drawn| drawn.0);
    let ground = entity.get::<Aloft>().map_or_else(
        || {
            world
                .get::<AreaTag>(entity.id())
                .map_or(Meters(0.0), |tag| {
                    area::load(world.resource::<Content>(), tag.area)
                        .heights
                        .height_at(drawn)
                })
        },
        |aloft| aloft.0,
    );
    let appearance = entity.get::<Appearance>()?;
    let built = ActorModel::get(world.resource::<Content>(), appearance.model);
    let stance = held::stance_of(world, entity.id());
    let shown = entity
        .get::<Puppet>()
        .and_then(Puppet::shown)
        .cloned()
        .or_else(|| {
            entity
                .get::<Playing>()
                .filter(|playing| playing.motion.shows_over(actor.action))
                .map(|playing| playing.motion.clone())
        })
        .unwrap_or(Motion::Action(actor.action));
    let pick = entity
        .get::<Puppet>()
        .map_or(0, |puppet| puppet.pick_of(&shown));
    let resolved = built.resolve(stance, &shown, actor.pose, pick);
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
    Some(ProbeDrawnBody {
        id: entity.id(),
        drawn,
        facing: entity
            .get::<RenderActor>()
            .map_or(actor.facing, |shown| shown.facing),
        naming: built.naming(stance, &shown, actor.pose),
        rate: built.rate(&shown, &resolved, actor.attack_rate, actor.speed),
        clip: resolved.clip.clone(),
        shown,
        feet: view.and_then(|view| view.window_point(drawn, ground)),
        head: view.and_then(|view| view.window_point(drawn, ground + capsule.height)),
        held,
        layered: entity.get::<Layered>().is_some_and(Layered::moving),
    })
}

fn drawn_items(world: &World, items: &[ProbeGroundItem]) -> Vec<ProbeDrawnItem> {
    items
        .iter()
        .map(|item| ProbeDrawnItem {
            id: item.id,
            plate: item::plates::plate_of(world, item.id).map(gui::window_rect),
            effects: world.get::<StandingVfx>(item.id).is_some(),
            outlined: world.get::<Outline>(item.id).is_some(),
        })
        .collect()
}

fn card_material(content: &Content, style: Option<CardStyleId>) -> CardMaterial {
    CardStyleDef::of(content, style).material.clone()
}

/// The pixel to rest the pointer on to take `thing`: an object by its shape, what has a body by
/// its body, and what has neither where it lies; none for what the client shows no such thing of,
/// or what shows nowhere in the window.
fn aim_at(world: &mut World, thing: Entity) -> Option<Pos<WindowPx>> {
    let view = render::view_of(world)?;
    if let Some(hitbox) = world
        .get::<AreaObject>(thing)
        .map(|object| object.shape.bounds())
    {
        return object_aim(world, &view, thing, hitbox);
    }
    let at = world.get::<Position>(thing)?.pos;
    match world.get::<Hitbox>(thing).map(|hitbox| hitbox.0) {
        Some(capsule) => body_aim(world, &view, thing, at, capsule),
        None => pixel_of(&view, world_point(at, ground_height(world, at))),
    }
}

/// The pixel to point at to take the body: over its capsule, at the height nearest its middle
/// where pointing claims the body, else over where it stands. A body is given up on at the first
/// height where pointing takes nothing at all: nothing stands in its way there, so it is not for
/// taking.
fn body_aim(
    world: &mut World,
    view: &View,
    entity: Entity,
    at: Pos<Meters>,
    capsule: Capsule,
) -> Option<Pos<WindowPx>> {
    const HEIGHTS: [f32; 7] = [0.5, 0.375, 0.625, 0.25, 0.75, 0.125, 0.875];
    let foot = actor::foot_height(world, entity, at);
    for share in HEIGHTS {
        let Some(pixel) = pixel_of(
            view,
            world_point(at, foot + Meters(capsule.height.0 * share)),
        ) else {
            continue;
        };
        match clicked(world, &pointed_through(world, view, pixel)) {
            Some(clicked) if clicked == entity => return Some(pixel),
            Some(_) => {}
            None => break,
        }
    }
    pixel_of(view, world_point(at, foot))
}

/// The pixel to point at to take the object: over the middle of its shape, or over the nearest
/// point of it where pointing claims the object, else over the middle.
fn object_aim(
    world: &mut World,
    view: &View,
    entity: Entity,
    hitbox: Rect<Meters>,
) -> Option<Pos<WindowPx>> {
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
    let over = |spot: Pos<Meters>| pixel_of(view, world_point(spot, ground_height(world, spot)));
    let pixels: Vec<Pos<WindowPx>> = std::iter::once(center)
        .chain(spots)
        .filter_map(over)
        .collect();
    let middle = over(center);
    pixels
        .into_iter()
        .find(|&pixel| clicked(world, &pointed_through(world, view, pixel)) == Some(entity))
        .or(middle)
}

/// Where each of `spots` shows to be clicked as a click reads the window now: the pixel over the
/// spot, on what lies open to the sky there or on the ground under it, whose click lands no
/// farther from the spot than the ground the pixel spans; none where no such pixel lies in the
/// window, for a spot hidden from the eye or one a click there takes something else at.
pub fn show(world: &mut World, spots: &[Pos<Meters>]) -> Vec<Option<ProbeShown>> {
    let Some(view) = render::view_of(world) else {
        return vec![None; spots.len()];
    };
    spots
        .iter()
        .map(|&spot| {
            let surface = surface_height(world, spot);
            let ground = ground_height(world, spot);
            let heights = if surface == ground {
                vec![ground]
            } else {
                vec![surface, ground]
            };
            heights.into_iter().find_map(|height| {
                let pixel = pixel_of(&view, world_point(spot, height))?;
                let lands = landing(world, &view, pixel)?;
                let span = span(world, &view, pixel)?;
                (lands.distance_to(spot) <= span.0).then_some(ProbeShown {
                    at: pixel,
                    lands,
                    span,
                })
            })
        })
        .collect()
}

/// Where a click at `pixel` goes or aims, if it does either.
fn landing(world: &mut World, view: &View, pixel: Pos<WindowPx>) -> Option<Pos<Meters>> {
    match claims::claim_at(world, Some(&pointed_through(world, view, pixel))) {
        ClickClaim::Walk(at) | ClickClaim::AimAt(at) => Some(at),
        _ => None,
    }
}

/// How much ground `pixel` spans: how far the ground under it lies from the ground under the
/// pixels beside it, the farther of the two.
fn span(world: &World, view: &View, pixel: Pos<WindowPx>) -> Option<Meters> {
    let under = |pixel: Pos<WindowPx>| pointed_through(world, view, pixel).surface;
    let here = under(pixel)?;
    [Offset::new(1.0, 0.0), Offset::new(0.0, 1.0)]
        .into_iter()
        .filter_map(|step| under(pixel + step))
        .map(|beside| Meters(beside.distance_to(here)))
        .reduce(|a, b| if b.0 > a.0 { b } else { a })
}

/// The whole pixel of the window a point of the world shows in, none outside the window.
fn pixel_of(view: &View, point: Vec3) -> Option<Pos<WindowPx>> {
    let shown = view.project(point)?.round();
    let inside = shown.cmpge(Vec2::ZERO).all() && shown.cmplt(view.viewport()).all();
    inside.then(|| Pos::new(shown.x, shown.y))
}

/// The pointer as it points through `pixel` of the window.
fn pointed_through(world: &World, view: &View, pixel: Pos<WindowPx>) -> Pointed {
    Pointed::along(world, view.ray(Vec2::new(pixel.x, pixel.y)))
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
                        .filter(|label| !label.is_empty())
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
            let editable = matches!(
                role,
                accesskit::Role::TextInput | accesskit::Role::MultilineTextInput
            );
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
            | Role::MultilineTextInput
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

fn map_marks(world: &World) -> Vec<ProbeMapMark> {
    let Some(shown) = world.get_resource::<area_map::MarksShown>() else {
        return Vec::new();
    };
    let mut marks: Vec<ProbeMapMark> = shown
        .marks
        .iter()
        .flat_map(|(view, marks)| {
            marks.iter().map(|mark| ProbeMapMark {
                view: (*view).to_owned(),
                actor: mark.who,
                at: Pos::new(mark.at.x, mark.at.y),
                marks: mark.marks.clone(),
            })
        })
        .collect();
    marks.sort_by(|a, b| a.view.cmp(&b.view));
    marks
}

fn map_rings(world: &World) -> Vec<ProbeMapRing> {
    let Some(shown) = world.get_resource::<area_map::MarksShown>() else {
        return Vec::new();
    };
    let mut rings: Vec<ProbeMapRing> = shown
        .rings
        .iter()
        .map(|(view, ring)| ProbeMapRing {
            view: (*view).to_owned(),
            actor: ring.who,
            marks: ring.marks.clone(),
            open: ring.open,
        })
        .collect();
    rings.sort_by(|a, b| a.view.cmp(&b.view));
    rings
}

fn map_tiles(world: &World) -> ProbeMapTiles {
    let tiles = world
        .get_resource::<area_map::pictures::MapPictures>()
        .map(|map| map.tiles())
        .unwrap_or_default();
    ProbeMapTiles {
        wanted: tiles.wanted,
        shown: tiles.shown,
        ahead: tiles.ahead,
    }
}

fn bag_views(world: &World) -> Vec<ProbeBagView> {
    let Some(shown) = world.get_resource::<bag_view::BagsShown>() else {
        return Vec::new();
    };
    shown
        .views
        .iter()
        .map(|(&placement, bag)| ProbeBagView {
            placement,
            words: bag.filter.words.clone(),
            category: bag.filter.category,
            shown: bag.slots.clone(),
        })
        .collect()
}

fn bag_carried(world: &World) -> Option<ProbeBagCarried> {
    let carried = world
        .get_resource::<bag_view::BagsShown>()?
        .carried
        .clone()?;
    Some(ProbeBagCarried {
        from: carried.from,
        over: carried.over,
        does: carried.does,
    })
}
