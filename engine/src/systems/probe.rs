use std::collections::HashMap;
use ui::egui;
use ui::float;

use bevy::diagnostic::FrameCount;
use bevy::input::ButtonState;
use bevy::input::keyboard::KeyboardInput;
use bevy::input::mouse::{MouseButtonInput, MouseWheel};
use bevy::prelude::*;
use bevy::render::renderer::RenderDevice;
use bevy::window::{CursorOptions, PrimaryWindow};
use bevy_egui::{EguiOutput, EguiPostUpdateSet, PrimaryEguiContext};
use serde::{Deserialize, Serialize};

use crate::core::audio::mix::AudioCategory;
use crate::core::audio::playback::Listener;
use crate::core::audio::soundscape::{self, SoundscapeMixer, SoundscapeShape};
use crate::core::content::{Content, ContentScope, with_content};
use crate::core::geometry::{Capsule, Radians};
use crate::core::gui::{self, GuiContext, WorldLayers};
use crate::core::math::{Offset, Pos, Rect, Size, WindowPx};
use crate::core::platform::ClientPlatform;
use crate::core::render::transition::{ActiveTransition, ScreenTransitionPhase};
use crate::core::render::world::layers::Layered;
use crate::core::render::world::sky::Daylight;
use crate::core::render::{self, CameraRig, GroundHeights, Pointed, ground_height, world_point};
use crate::core::time::{self, PlaybackRate, Seconds};
use crate::core::units::{Bytes, GROUND_ORIGIN, Meters};
use crate::core::vfx::{VfxId, VfxInstance, VfxSite};
use crate::systems::actor::held::{self, Held};
use crate::systems::actor::render::Puppet;
use crate::systems::actor::skeleton::{Clip, SkeletonId};
use crate::systems::actor::stance::{HandIndex, StanceId};
use crate::systems::actor::{self, Action, Actor, ActorModel, Appearance, Hitbox, Motion, Playing};
use crate::systems::area::{self, AreaId, AreaObject, AreaTag};
use crate::systems::attention::{Attention, AttentionId, StatusBadges};
use crate::systems::combat::Attitude;
use crate::systems::dialogue::stage;
use crate::systems::history::widget::{self as history, HistoryBook};
use crate::systems::history::{HistoryMark, HistoryRecord, HistoryTopic};
use crate::systems::hud;
use crate::systems::input::gestures::{self, ClickClaim};
use crate::systems::input::map::InputMap;
use crate::systems::item::{self, DroppedItem, Inventory, ItemId};
use crate::systems::movement::{Position, RenderPosition};
use crate::systems::notification::{self, NotificationRows, bubble, error, feed, intro, pop};
use crate::systems::npc::{Npc, NpcId};
use crate::systems::player::{PlayedBy, commands_locked, session};
use crate::systems::quest::{Parties, QuestId, QuestLog, QuestResult};
use crate::systems::script::{OwnerRow, Reach, RiftScripts};
use crate::systems::shop::{self, ShopId};
use crate::systems::skill::{self, Rank, SkillId, bar::SlotReport};
use crate::systems::stat::{StatKind, Stats};
use crate::systems::status::{self, StatusCategory, StatusEffectId};

/// How a driver asks the client what it shows: the frame after a driver asks answers it.
pub trait ProbeChannel {
    fn awaited(&self) -> bool;
    fn deliver(&self, snapshot: &str);
}

pub fn install(app: &mut App, channel: impl ProbeChannel + 'static) {
    app.insert_non_send(Probe(Box::new(channel)))
        .init_resource::<ProbeInputSeen>()
        .init_resource::<TerrainSent>()
        .init_resource::<DrawnUi>()
        .init_resource::<VfxSeen>()
        .add_observer(remember_vfx)
        .add_systems(PreUpdate, (count_input, read_accessibly))
        .add_systems(
            PostUpdate,
            (copy_drawn_ui, count_clicks).after(EguiPostUpdateSet::ProcessOutput),
        )
        .add_systems(Last, publish);
}

/// One frame of the client. Content ids (de)serialize by key, so a driver reads a snapshot within a
/// content scope. World positions are metres on the ground, which `view` and `terrain` place in the
/// window; window positions are the window's logical pixels.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProbeSnapshot {
    pub frame: u32,
    pub input: ProbeInput,
    pub claim: ClickClaim,
    pub screen: Option<Size<WindowPx>>,
    pub view: Option<ProbeView>,
    pub area: Option<AreaId>,
    /// The area's ground heights, sent with the first frame in a new area and left out after, so a
    /// driver keeps the last it saw.
    pub terrain: Option<ProbeTerrain>,
    pub me: Option<ProbeBody>,
    pub actors: Vec<ProbeBody>,
    pub objects: Vec<ProbeObject>,
    pub items: Vec<ProbeGroundItem>,
    pub warps: Vec<ProbeWarp>,
    /// What the interface shows: text, icons, fields and sliders, less what is scrolled or clipped
    /// out of sight.
    pub ui: Vec<ProbeUiElement>,
    /// Where the interface takes the pointer: each rect edges included, as egui hit-tests.
    pub covered: Vec<Rect<WindowPx>>,
    pub stage: Option<ProbeStage>,
    pub notifications: ProbeNotifications,
    pub feed: Vec<String>,
    pub pops: Vec<pop::ShownPop>,
    pub history: ProbeHistory,
    pub shop: Option<ProbeShop>,
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
    pub transition: ScreenTransitionPhase,
    pub daylight: Daylight,
    /// What the GPU holds in buffers and textures for the client.
    pub gpu_memory: Bytes,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct ProbeInput {
    pub cursor: Option<Pos<WindowPx>>,
    /// Whether the system draws the cursor, rather than the frame or nothing.
    pub system_cursor: bool,
    /// Whether a click now would still count on from the latest one, as a double or triple click.
    pub click_run_open: bool,
    #[serde(flatten)]
    pub seen: ProbeInputSeen,
}

/// What the driver has sent so far, as the game received it: a driver that counts what it sends
/// knows when the game has caught up, however slowly frames come.
#[derive(Resource, Serialize, Deserialize, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub struct ProbeInputSeen {
    pub mouse_presses: u32,
    pub mouse_releases: u32,
    /// How many clicks in a row the game counted the latest one as.
    pub click_run: u8,
    pub wheel_turns: u32,
    pub key_presses: u32,
}

impl ProbeSnapshot {
    /// Where a point on the ground shows in the window, at the terrain's height there: nowhere
    /// while the ground rises between it and the camera.
    pub fn ground_point(&self, at: Pos<Meters>) -> Option<Pos<WindowPx>> {
        let view = self.view?;
        let Some(terrain) = &self.terrain else {
            return Some(view.window_point(at, Meters(0.0)));
        };
        let height = terrain.height_at(at);
        let ground = Vec3::new(at.x, height.0, at.y);
        (!terrain.rises_between(view.near_point(ground), ground))
            .then(|| view.window_point(at, height))
    }
}

/// The camera: how a world point (x east, height up, y south) lands in the window, and where the
/// camera looks from.
#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct ProbeView {
    pub view_proj: Mat4,
    pub viewport: Rect<WindowPx>,
    pub yaw: Radians,
    pub pitch: Radians,
    pub zoom: u8,
}

impl ProbeView {
    /// Where the camera's ray to a world point sets out: on the near plane, in front of it.
    fn near_point(&self, point: Vec3) -> Vec3 {
        let clip = self.view_proj * point.extend(1.0);
        let ndc = clip.truncate() / clip.w;
        let near = self.view_proj.inverse() * Vec4::new(ndc.x, ndc.y, 1.0, 1.0);
        near.truncate() / near.w
    }

    pub fn window_point(&self, at: Pos<Meters>, height: Meters) -> Pos<WindowPx> {
        let clip = self.view_proj * Vec4::new(at.x, height.0, at.y, 1.0);
        let ndc = clip.truncate() / clip.w;
        self.viewport.origin
            + Offset::new(
                (ndc.x + 1.0) / 2.0 * self.viewport.size.width,
                (1.0 - ndc.y) / 2.0 * self.viewport.size.height,
            )
    }
}

/// An area's ground heights at whole metres, from its corner on.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ProbeTerrain {
    pub size: Size<Meters>,
    pub heights: Vec<Meters>,
}

impl ProbeTerrain {
    /// Whether the ground stands over the straight line from `from` to the ground point `to`
    /// anywhere short of it.
    fn rises_between(&self, from: Vec3, to: Vec3) -> bool {
        let run = Vec2::new(to.x - from.x, to.z - from.z).length();
        let steps = (run / SIGHT_STEP.0).ceil() as usize;
        (1..steps)
            .map(|step| from.lerp(to, step as f32 / steps as f32))
            .take_while(|point| Vec2::new(to.x - point.x, to.z - point.z).length() > SIGHT_STEP.0)
            .any(|point| self.height_at(Pos::new(point.x, point.z)).0 > point.y + SIGHT_SLACK.0)
    }

    pub fn height_at(&self, at: Pos<Meters>) -> Meters {
        let columns = self.size.width as usize + 1;
        let rows = self.size.height as usize + 1;
        let x = (at.x - GROUND_ORIGIN.x).clamp(0.0, columns as f32 - 1.0);
        let y = (at.y - GROUND_ORIGIN.y).clamp(0.0, rows as f32 - 1.0);
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

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProbeBody {
    pub id: Entity,
    pub name: String,
    pub appearance: Appearance,
    pub flies: bool,
    pub player: bool,
    pub at: Pos<Meters>,
    /// Where it shows on screen, which runs behind `at` for others and ahead of it for the player's
    /// own walk.
    pub drawn: Pos<Meters>,
    /// Where a click takes it rather than whatever stands in front of it.
    pub aim: Pos<Meters>,
    pub capsule: Capsule,
    pub facing: Radians,
    pub action: Action,
    /// Whether it stands still rather than walks.
    pub resting: bool,
    /// The motion its body shows: what it does, or a move, a gesture, a flinch at a blow that
    /// just landed on it.
    pub shown: Motion,
    /// Where its feet and the top of its head show in the window, as drawn.
    pub feet: Option<Pos<WindowPx>>,
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
    /// What each hand of its body shows, as the server says.
    pub wearing: Vec<(HandIndex, ItemId)>,
    /// The props its rig holds as drawn, each where it shows in the window.
    pub held: Vec<ProbeHeld>,
    /// Whether any layer moves its joints this frame.
    pub layered: bool,
}

/// A prop hanging from a hand of a drawn body.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProbeHeld {
    pub hand: HandIndex,
    pub item: ItemId,
    pub at: Option<Pos<WindowPx>>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProbeObject {
    pub id: Entity,
    pub name: Option<String>,
    pub title: Option<String>,
    pub at: Pos<Meters>,
    pub aim: Pos<Meters>,
    pub hitbox: Rect<Meters>,
    pub marks: Vec<AttentionId>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProbeGroundItem {
    pub id: Entity,
    pub item: ItemId,
    pub count: u32,
    pub at: Pos<Meters>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProbeWarp {
    pub name: String,
    pub to: AreaId,
    pub at: Pos<Meters>,
    pub rect: Rect<Meters>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct ProbeUiElement {
    pub text: Option<String>,
    pub image: Option<String>,
    pub editable: bool,
    pub button: bool,
    pub focused: bool,
    pub click_through: bool,
    pub in_world: bool,
    pub slider: Option<f32>,
    pub rect: Rect<WindowPx>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProbeStage {
    pub node: String,
    /// The dialogue box as last drawn.
    pub rect: Option<Rect<WindowPx>>,
    pub with: Option<Entity>,
    pub line: usize,
    pub lines: usize,
    pub speaker: Option<String>,
    pub text: Option<String>,
    pub typing: bool,
    pub choices: Vec<ProbeChoice>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProbeChoice {
    pub label: String,
    pub locked: bool,
    pub selected: bool,
    pub rect: Option<Rect<WindowPx>>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProbeNotifications {
    pub bubbles: Vec<ProbeBubble>,
    pub edge_folds: Vec<u32>,
    pub captions: NotificationRows,
    pub alerts: NotificationRows,
    pub error: Option<String>,
    pub milestones: NotificationRows,
    pub intro: Option<ProbeIntro>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProbeBubble {
    pub speaker: String,
    pub npc: NpcId,
    pub body: Option<Entity>,
    pub replaced: u32,
    pub lines: Vec<String>,
    pub folded: bool,
    pub rect: Option<Rect<WindowPx>>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProbeIntro {
    pub title: String,
    pub text: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProbeHistory {
    pub open: bool,
    pub tab: Option<String>,
    pub records: Vec<ProbeRecord>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProbeRecord {
    pub topic: HistoryTopic,
    pub by: Option<String>,
    pub text: String,
    pub mark: Option<HistoryMark>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProbeShop {
    pub shop: ShopId,
    pub offers: Vec<ProbeOffer>,
    pub buyback: Vec<ProbeStack>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProbeOffer {
    pub item: ItemId,
    pub count: u32,
    pub left: Option<u32>,
    pub refusal: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct ProbeStack {
    pub item: ItemId,
    pub count: u32,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct ProbeItemCard {
    pub item: ItemId,
    pub rect: Rect<WindowPx>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct ProbeSkillCard {
    pub skill: SkillId,
    pub rect: Rect<WindowPx>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ProbeStatus {
    pub effect: StatusEffectId,
    pub category: StatusCategory,
    pub left: Option<Seconds>,
}

/// A vfx seen starting lately, with the site it plays on; `over` once its time has run out. One
/// shorter than the gap between two snapshots is still reported in the next.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ProbeVfx {
    pub vfx: VfxId,
    pub site: VfxSite,
    pub over: bool,
}

/// The character's class, tree and bar as the client shows them.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ProbeSkills {
    pub class: String,
    pub level: u32,
    pub to_spend: u32,
    pub learned: Vec<(String, Rank)>,
    pub planned: Vec<(String, Rank)>,
    pub aiming: Option<SkillId>,
    pub reset_prompt: bool,
    /// The skills window, while open.
    pub view: Option<ProbeTreeView>,
}

/// How the skills window shows the tree: the canvas (backdrop and nodes) pans within the
/// viewport, which the canvas always fills, and the window stops growing at `max`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ProbeTreeView {
    pub window: Rect<WindowPx>,
    pub max: Option<Size<WindowPx>>,
    pub viewport: Rect<WindowPx>,
    pub canvas: Size<WindowPx>,
    pub panned: Offset<WindowPx>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ProbeQuests {
    pub active: Vec<ProbeQuest>,
    pub finished: Vec<ProbeFinishedQuest>,
    pub tracked: Vec<QuestId>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProbeQuest {
    pub quest: QuestId,
    pub ready: bool,
    pub left: Option<Seconds>,
    /// Each objective's [have, need].
    pub progress: Vec<[u32; 2]>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct ProbeFinishedQuest {
    pub quest: QuestId,
    pub result: QuestResult,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProbeSoundscape {
    pub zones: Vec<ProbeSoundZone>,
    pub voices: Vec<ProbeVoice>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProbeSoundZone {
    pub bounds: Rect<Meters>,
    pub ellipse: bool,
    pub reach: Meters,
    pub proximity: f32,
    pub tracks: Vec<ProbeTrack>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProbeTrack {
    pub channel: u32,
    pub src: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProbeVoice {
    pub channel: u32,
    pub category: AudioCategory,
    pub src: String,
    pub heard: f32,
}

struct Probe(Box<dyn ProbeChannel>);

#[derive(Resource, Default)]
struct DrawnUi(Option<accesskit::TreeUpdate>);

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
    let Some(gui) = gui else {
        return;
    };
    let (run, released) = gui.0.input(|input| {
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

// egui counts a click as the third of a run while the click two before it came within twice the
// double click delay, wherever that click landed.
fn click_run_open(world: &World) -> bool {
    world.get_resource::<GuiContext>().is_some_and(|gui| {
        let delay = gui
            .0
            .options(|options| options.input_options.max_double_click_delay);
        let since = gui.0.input(|input| input.pointer.time_since_last_click());
        f64::from(since) < 2.0 * delay
    })
}

// A snapshot scans the whole world, so only a frame the driver is waiting for pays for one.
fn publish(world: &mut World) {
    if !world.contains_resource::<Content>() || !world.non_send::<Probe>().0.awaited() {
        return;
    }
    let snapshot = snapshot(world);
    let scope = ContentScope::of(world.resource::<Content>());
    let (json, _) = with_content(scope, || {
        serde_json::to_string(&snapshot).expect("a serializable snapshot")
    });
    world.non_send::<Probe>().0.deliver(&json);
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
    ProbeSnapshot {
        frame: world.resource::<FrameCount>().0,
        input: ProbeInput {
            cursor: window
                .as_ref()
                .and_then(Window::cursor_position)
                .map(|at| Pos::new(at.x, at.y)),
            system_cursor,
            click_run_open: click_run_open(world),
            seen: *world.resource::<ProbeInputSeen>(),
        },
        screen: window
            .as_ref()
            .map(|window| Size::new(window.resolution.width(), window.resolution.height())),
        claim: gestures::claim(world),
        stage: stage::view(world).map(|view| ProbeStage {
            node: view.node,
            rect: stage::dialogue_rect(world).map(window_rect),
            with: view.with,
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
                    locked: choice.locked,
                    selected: choice.selected,
                    rect: rows.get(&index).copied().map(window_rect),
                })
                .collect(),
        }),
        notifications: notifications(world),
        feed: feed::rows(world),
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
            rect: window_rect(rect),
        }),
        skill_card: skill::card::shown(world)
            .zip(hud::pinned_rect::<SkillId>(world))
            .map(|(skill, rect)| ProbeSkillCard {
                skill,
                rect: window_rect(rect),
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
        terrain: terrain(world, area),
        actors: actors(world, me, area),
        objects: objects(world, area),
        items: items(world, area),
        warps: warps(world, area),
        ui: ui(world),
        covered: covered(world),
        soundscape: soundscape(world),
        transition: world.resource::<ActiveTransition>().phase(),
        daylight: world
            .get_resource::<Daylight>()
            .copied()
            .unwrap_or_default(),
        gpu_memory: gpu_memory(world),
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

fn soundscape(world: &World) -> ProbeSoundscape {
    let listener = world.resource::<Listener>().0;
    ProbeSoundscape {
        zones: world
            .resource::<soundscape::Soundscape>()
            .0
            .iter()
            .map(|zone| ProbeSoundZone {
                bounds: zone.shape.bounds(),
                ellipse: matches!(zone.shape, SoundscapeShape::Ellipse(_)),
                reach: zone.reach(),
                proximity: listener
                    .and_then(|at| zone.heard_from(at))
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
        .map(|learned| learned.0.iter().map(|(k, r)| (k.to_string(), *r)).collect())
        .unwrap_or_default();
    Some(ProbeSkills {
        class: class.path(content).to_owned(),
        level: crate::systems::class::level(world, me),
        to_spend: crate::systems::class::points_to_spend(world, me).0,
        learned,
        planned: world
            .resource::<skill::window::Plan>()
            .0
            .iter()
            .map(|(k, r)| (k.to_string(), *r))
            .collect(),
        aiming: skill::aim::aiming(world).map(|aimed| aimed.skill),
        reset_prompt: skill::prompt::showing(world),
        view: tree_view(world),
    })
}

fn tree_view(world: &World) -> Option<ProbeTreeView> {
    let window = hud::window_rect(world, &skill::window::SkillsWindow)?;
    let view = world.resource::<skill::window::TreeView>();
    Some(ProbeTreeView {
        window: window_rect(window),
        max: hud::window_limit(world, &skill::window::SkillsWindow)
            .map(|max| Size::new(max.x, max.y)),
        viewport: window_rect(view.viewport),
        canvas: Size::new(view.content.x, view.content.y),
        panned: Offset::new(view.offset.x, view.offset.y),
    })
}

fn quests(world: &World, seen: Entity) -> Option<ProbeQuests> {
    let content = world.resource::<Content>();
    let log = world.get::<QuestLog>(seen)?;
    let inventory = world.get::<Inventory>(seen);
    let now = world.resource::<ClientPlatform>().0.local_clock().now;
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
    })
}

/// Which area's terrain the probe last sent, so it goes out once per arrival.
#[derive(Resource, Default)]
struct TerrainSent(Option<AreaId>);

fn terrain(world: &mut World, area: Option<AreaId>) -> Option<ProbeTerrain> {
    let mut sent = world.resource_mut::<TerrainSent>();
    if sent.0 == area {
        return None;
    }
    sent.0 = area;
    let content = world.resource::<Content>();
    let area = area::load(content, area?);
    let size = area.size;
    let (columns, rows) = (size.width as usize + 1, size.height as usize + 1);
    let heights = (0..rows)
        .flat_map(|row| (0..columns).map(move |column| (column, row)))
        .map(|(column, row)| {
            area.heights
                .height_at(GROUND_ORIGIN + Offset::new(column as f32, row as f32))
        })
        .collect();
    Some(ProbeTerrain { size, heights })
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
    let clip = resolved.clip.clone();
    let rate = built.rate(&shown, &resolved, actor.attack_rate, actor.speed);
    let held = entity
        .get::<Puppet>()
        .map(|puppet| {
            puppet
                .held()
                .map(|(hand, item, prop)| ProbeHeld {
                    hand,
                    item,
                    at: world.get::<GlobalTransform>(prop).and_then(|placed| {
                        let at = placed.translation();
                        view.map(|view| view.window_point(Pos::new(at.x, at.z), Meters(at.y)))
                    }),
                })
                .collect()
        })
        .unwrap_or_default();
    Some(ProbeBody {
        id: entity.id(),
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
        facing: actor.facing,
        action: actor.action,
        resting: !actor.action.moves(),
        shown: shown.clone(),
        feet: view.map(|view| view.window_point(drawn, ground)),
        head: view.map(|view| view.window_point(drawn, ground + capsule.height)),
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
        wearing: entity
            .get::<Held>()
            .map(|wearing| {
                wearing
                    .hands
                    .iter()
                    .map(|(&hand, &item)| (hand, item))
                    .collect()
            })
            .unwrap_or_default(),
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
            };
            (entity, seen)
        })
        .collect();
    objects
        .into_iter()
        .map(|(entity, seen)| ProbeObject {
            aim: object_aim(world, entity, seen.hitbox),
            ..seen
        })
        .collect()
}

/// The ground point to point at to take the body: where the ray from the eye through the middle of
/// its capsule lands, when pointing there claims the body, else where it stands.
fn aim(world: &mut World, entity: Entity, at: Pos<Meters>, capsule: Capsule) -> Pos<Meters> {
    let Some(view) = render::view_of(world) else {
        return at;
    };
    let middle = world_point(
        at,
        ground_height(world, at) + Meters(capsule.height.0 / 2.0),
    );
    let eye = view.eye();
    let Ok(direction) = Dir3::new(middle - eye) else {
        return at;
    };
    let ray = Ray3d::new(eye, direction);
    let ground = world.resource::<GroundHeights>().hit(ray);
    let pointed = Pointed { ray, ground };
    match (clicked(world, &pointed), ground) {
        (Some(clicked), Some(ground)) if clicked == entity => ground,
        _ => at,
    }
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
    match gestures::claim_at(world, Some(pointed)) {
        ClickClaim::Attack(entity) | ClickClaim::Interact(entity) | ClickClaim::Aim(entity) => {
            Some(entity)
        }
        _ => None,
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
                    Reach::Warp(place) => Some(*place),
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
    let world_layers = world.resource::<WorldLayers>();
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
                click_through: float::layer_at(&gui, rect.center()) != Some(widget.layer_id),
                in_world: world_layers.contains(widget.layer_id),
                slider,
                rect: window_rect(rect),
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
    let content = world.resource::<Content>().clone();
    let bubbles = bubble::shown(world)
        .into_iter()
        .map(|shown| ProbeBubble {
            speaker: shown.speaker.get(&content).name.to_owned(),
            npc: shown.speaker,
            body: shown.body,
            replaced: shown.replaced,
            lines: shown.lines,
            folded: shown.folded,
            rect: shown.rect.map(window_rect),
        })
        .collect();
    let edge_folds = bubble::edge_folds(world);
    ProbeNotifications {
        bubbles,
        edge_folds,
        captions: notification::captions(world),
        alerts: notification::alerts(world),
        error: error::shown(world),
        milestones: notification::milestones(world),
        intro: intro::shown(world).map(|(title, text)| ProbeIntro { title, text }),
    }
}

fn window_rect(rect: egui::Rect) -> Rect<WindowPx> {
    Rect::new(
        Pos::new(rect.min.x, rect.min.y),
        Size::new(rect.width(), rect.height()),
    )
}

fn covered(world: &mut World) -> Vec<Rect<WindowPx>> {
    let Some(gui) = gui::context(world) else {
        return Vec::new();
    };
    let layers: Vec<egui::LayerId> =
        gui.memory(|memory| memory.areas().visible_layer_ids().into_iter().collect());
    // egui hands a click to a widget this close to it as well as one under it.
    let reach = gui.global_style().interaction.interact_radius;
    layers
        .into_iter()
        .filter(|&layer| layer != egui::LayerId::background())
        .filter_map(|layer| egui::AreaState::load(&gui, layer.id))
        .filter(|area| area.interactable)
        .map(|area| area.rect().expand(reach))
        .filter(|rect| rect.width() > 0.0 && rect.height() > 0.0)
        .map(window_rect)
        .collect()
}

const VFX_MEMORY: Seconds = Seconds(3.0);
/// How finely the line of sight to a ground point is walked, and how far the ground may stand over
/// it before it hides the point: the probe's heights lie a metre apart, coarser than the ground
/// the GPU draws.
const SIGHT_STEP: Meters = Meters(0.5);
const SIGHT_SLACK: Meters = Meters(0.3);

#[derive(Resource, Default)]
struct VfxSeen(Vec<VfxInstance>);

fn remember_vfx(
    add: On<Add<VfxInstance>>,
    instances: Query<&VfxInstance>,
    mut seen: ResMut<VfxSeen>,
) {
    if let Ok(instance) = instances.get(add.entity) {
        seen.0.push(instance.clone());
    }
}

fn vfx_seen(world: &mut World) -> Vec<ProbeVfx> {
    let now = time::server_now(world);
    let mut seen = world.resource_mut::<VfxSeen>();
    if let Some(now) = now {
        seen.0
            .retain(|instance| now < instance.until.after(VFX_MEMORY));
    }
    seen.0
        .iter()
        .map(|instance| ProbeVfx {
            vfx: instance.vfx,
            site: instance.site.clone(),
            over: now.is_some_and(|now| instance.until <= now),
        })
        .collect()
}
