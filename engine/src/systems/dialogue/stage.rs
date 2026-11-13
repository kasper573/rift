use std::collections::HashMap;
use std::time::Duration;
use ui::egui;
use ui::float;

use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use ui::controls::{self, Pill, RowHover, RowLook};
use ui::egui::{
    Align, Align2, Color32, Id, LayerId, Layout, Pos2, Rect, Sense, Ui, UiBuilder, Vec2,
    WidgetInfo, WidgetType,
};
use ui::face::TextRole;
use ui::glyph::{self, Mark};
use ui::motion::SheetPose;
use ui::sheet::{self, Sheet};
use ui::{ink, slip};

use super::camera::DialogueCamera;
use super::{
    ChoiceChip, ChoiceView, Conversation, ConversationRequest, ConversationStep, RefusedPick,
    Speaker, SpokenLine,
};
use crate::core::audio::playback::SfxPlace;
use crate::core::babble::{BabbleRank, Babbler, Babbling};
use crate::core::content::Content;
use crate::core::gui::{self, Band, Tier};
use crate::core::text::{self, StyledLine, TextLook, TextPiece, Typewriter};
use crate::core::time::{self};
use crate::core::units::MeterPos;
use crate::systems::actor::Name;
use crate::systems::actor::render::Said;
use crate::systems::hud;
use crate::systems::input::map::{self, InputAction, InputMap};
use crate::systems::interface::InterfaceSound;
use crate::systems::item::{ItemId, card};
use crate::systems::movement::Position;
use crate::systems::npc::Npc;
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;

const CHOICE_SHORTCUTS: [InputAction; 9] = [
    InputAction::Choice1,
    InputAction::Choice2,
    InputAction::Choice3,
    InputAction::Choice4,
    InputAction::Choice5,
    InputAction::Choice6,
    InputAction::Choice7,
    InputAction::Choice8,
    InputAction::Choice9,
];
const SHEET_WIDTH: f32 = 800.0;
const NAME_INSET: f32 = 40.0;
const SHEET_LIFT: f32 = 14.0;
const PANEL_GAP: f32 = 12.0;
const SCREEN_SHARE: f32 = 0.94;
const SCREEN_MARGIN: f32 = 8.0;
const HINT_ROOM: f32 = 22.0;
const CONTINUE_MARK: f32 = 12.0;
const CHOICE_ICON: f32 = 18.0;
const LINE_MARGIN: egui::Margin = egui::Margin::symmetric(8, 4);
const PILL_ICON: f32 = 14.0;
const REFUSED: Duration = Duration::from_millis(320);
const DIALOGUE: &str = "stage";
const DOCKED: &str = "docked";

pub struct DialogueStagePlugin;

impl Plugin for DialogueStagePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Stage>()
            .init_resource::<StageDrawn>()
            .init_resource::<StageSheets>()
            .add_systems(OnExit(ClientScreen::Area), forget_stage)
            .add_systems(
                Update,
                (follow_conversation, stage_keys, track_typing)
                    .chain()
                    .run_if(in_state(ClientScreen::Area)),
            )
            .add_systems(
                EguiPrimaryContextPass,
                draw_stage
                    .in_set(Band::Stage)
                    .run_if(in_state(ClientScreen::Area)),
            );
    }
}

pub struct StageView {
    pub node: String,
    pub with: Option<Entity>,
    pub camera: DialogueCamera,
    pub line: usize,
    pub lines: usize,
    pub speaker: Option<String>,
    pub text: Option<String>,
    pub typing: bool,
    pub choices: Vec<StageChoice>,
}

pub struct StageChoice {
    pub label: String,
    pub gets: Vec<ItemId>,
    pub locked: bool,
    pub selected: bool,
}

#[derive(Clone, Copy)]
pub struct DockedPanel {
    pub head: fn(&World) -> Option<PanelHead>,
    pub show: fn(&mut Ui, &mut World, f32, f32),
}

pub struct PanelHead {
    pub width: f32,
    pub chrome: PanelChrome,
}

pub enum PanelChrome {
    Card,
    Sheet { title: String, height: f32 },
}

pub fn dock(app: &mut App, panel: DockedPanel) {
    app.world_mut()
        .get_resource_or_init::<DockedPanels>()
        .0
        .push(panel);
}

pub fn view(world: &World) -> Option<StageView> {
    let shown = world.resource::<Stage>().shown.as_ref()?;
    Some(StageView {
        node: shown.node.clone(),
        with: shown.with,
        camera: shown.seen_through(),
        line: shown.line,
        lines: shown.lines.len(),
        speaker: shown
            .current()
            .and_then(|line| speaker_name(world, line.by)),
        text: shown
            .current()
            .map(|line| line.text.words(world.resource::<InputMap>())),
        typing: shown.typed_since.is_none(),
        choices: shown
            .choices
            .iter()
            .enumerate()
            .map(|(index, choice)| StageChoice {
                label: choice.label.words(world.resource::<InputMap>()),
                gets: choice
                    .chips
                    .iter()
                    .filter_map(|chip| match chip {
                        ChoiceChip::Gets { item, .. } => Some(*item),
                        _ => None,
                    })
                    .collect(),
                locked: choice.locked,
                selected: index == shown.selected,
            })
            .collect(),
    })
}

/// Who the conversation on show stages, for the camera it is seen through.
pub(super) struct Cast {
    pub camera: DialogueCamera,
    pub with: Option<Entity>,
    /// The body saying the line on show, when a body says it.
    pub speaker: Option<Entity>,
    /// The bodies saying any of the lines on show.
    pub speakers: Vec<Entity>,
}

pub(super) fn cast(world: &mut World) -> Option<Cast> {
    let shown = world.resource::<Stage>().shown.as_ref()?;
    let (camera, with) = (shown.seen_through(), shown.with);
    let current = shown.current().map(|line| line.by);
    let said_by: Vec<Speaker> = shown
        .lines
        .iter()
        .map(|line| line.by)
        .chain(current)
        .collect();
    let mut speakers: Vec<Entity> = said_by
        .into_iter()
        .filter_map(|by| body_of(world, by, with))
        .collect();
    speakers.sort_unstable();
    speakers.dedup();
    Some(Cast {
        camera,
        with,
        speaker: current.and_then(|by| body_of(world, by, with)),
        speakers,
    })
}

pub fn dialogue_rect(world: &World) -> Option<Rect> {
    world.resource::<StageDrawn>().dialogue
}

pub fn rects(world: &World) -> Vec<Rect> {
    let drawn = world.resource::<StageDrawn>();
    drawn
        .dialogue
        .into_iter()
        .chain(drawn.panels.clone())
        .collect()
}

pub fn choice_rects(world: &World) -> HashMap<usize, Rect> {
    world.resource::<StageDrawn>().choices.clone()
}

pub fn leavable(world: &World) -> bool {
    world.resource::<Stage>().shown.is_some()
}

pub fn layer() -> LayerId {
    sheet_layer(DIALOGUE)
}

pub fn leave(world: &mut World) -> bool {
    if !leavable(world) {
        return false;
    }
    let Some(step) = world
        .resource::<Stage>()
        .shown
        .as_ref()
        .map(|shown| shown.step)
    else {
        return false;
    };
    world.write_message(ConversationRequest::Leave { step });
    true
}

#[derive(Resource, Default)]
struct DockedPanels(Vec<DockedPanel>);

#[derive(Resource, Default)]
struct Stage {
    shown: Option<Shown>,
}

struct Shown {
    step: ConversationStep,
    node: String,
    with: Option<Entity>,
    line: usize,
    lines: Vec<SpokenLine>,
    choices: Vec<ChoiceView>,
    camera: DialogueCamera,
    picked: bool,
    typed_since: Option<Duration>,
    remark: Option<SpokenLine>,
    remarks_seen: u32,
    refusals_seen: u32,
    typing: Option<Entity>,
    selected: usize,
    pointed: Option<usize>,
    refused: Option<(usize, Duration)>,
}

#[derive(Resource, Default, Clone)]
struct StageDrawn {
    dialogue: Option<Rect>,
    panels: Vec<Rect>,
    choices: HashMap<usize, Rect>,
}

fn forget_stage(world: &mut World) {
    if let Some(shown) = world.resource_mut::<Stage>().shown.take()
        && let Some(line) = shown.typing
    {
        world.despawn(line);
    }
    *world.resource_mut::<StageDrawn>() = StageDrawn::default();
}

fn follow_conversation(world: &mut World) {
    let conversation = session::me(world)
        .and_then(|seen| world.get::<Conversation>(seen))
        .cloned();
    let Some(now) = conversation else {
        if world.resource::<Stage>().shown.is_some() {
            close(world);
        }
        return;
    };
    let mut stage = world.resource_mut::<Stage>();
    match stage.shown.as_mut() {
        None => {
            stage.shown = Some(Shown::of(now));
            InterfaceSound::UiOpen.play(world);
            show_line(world);
        }
        Some(shown) if shown.step != now.step && shown.lines != now.lines => {
            let typing = shown.typing.take();
            stage.shown = Some(Shown::of(now));
            if let Some(line) = typing {
                world.despawn(line);
            }
            show_line(world);
        }
        Some(shown) => {
            if shown.step != now.step {
                shown.step = now.step;
                shown.node = now.node.clone();
                shown.picked = false;
            }
            shown.camera = now.camera;
            let remark = now
                .remark
                .clone()
                .filter(|remark| remark.nth > shown.remarks_seen);
            let refused = now
                .refused
                .clone()
                .filter(|refused| refused.nth > shown.refusals_seen);
            if shown.choices != now.choices {
                shown.selected = first_open(&now.choices);
            }
            shown.choices = now.choices;
            if let Some(refused) = refused {
                shown.refusals_seen = refused.nth;
                shown.picked = false;
                refuse(world, refused);
            }
            let mut stage = world.resource_mut::<Stage>();
            let Some(shown) = stage.shown.as_mut() else {
                return;
            };
            if let Some(remark) = remark {
                shown.remarks_seen = remark.nth;
                shown.remark = Some(remark.line);
                show_line(world);
            }
        }
    }
}

impl Shown {
    fn of(now: Conversation) -> Shown {
        Shown {
            step: now.step,
            node: now.node,
            with: now.with,
            line: 0,
            lines: now.lines,
            selected: first_open(&now.choices),
            pointed: None,
            choices: now.choices,
            camera: now.camera,
            picked: false,
            typed_since: None,
            remarks_seen: now.remark.map_or(0, |remark| remark.nth),
            refusals_seen: now.refused.map_or(0, |refused| refused.nth),
            remark: None,
            typing: None,
            refused: None,
        }
    }

    fn seen_through(&self) -> DialogueCamera {
        self.current()
            .and_then(|line| line.camera)
            .unwrap_or(self.camera)
    }

    fn current(&self) -> Option<SpokenLine> {
        self.remark
            .clone()
            .or_else(|| self.lines.get(self.line).cloned())
    }

    /// Whether the conversation has anything to say or to pick: one held open only for a panel
    /// docked over it has not, and its panel stands alone.
    fn speaks(&self) -> bool {
        self.current().is_some() || !self.choices.is_empty()
    }

    fn last_line(&self) -> bool {
        self.line + 1 >= self.lines.len()
    }

    fn choosing(&self) -> bool {
        self.typed_since.is_some() && self.last_line() && !self.choices.is_empty()
    }
}

fn first_open(choices: &[ChoiceView]) -> usize {
    choices
        .iter()
        .position(|choice| !choice.locked)
        .unwrap_or_default()
}

fn close(world: &mut World) {
    forget_stage(world);
    InterfaceSound::UiClose.play(world);
}

fn show_line(world: &mut World) {
    let Some(line) = world
        .resource::<Stage>()
        .shown
        .as_ref()
        .and_then(Shown::current)
    else {
        return;
    };
    let styled = line.text.styled();
    let typing = world.spawn((styled, Typewriter::default())).id();
    let with = world
        .resource::<Stage>()
        .shown
        .as_ref()
        .and_then(|shown| shown.with);
    if let Some(by) = body_of(world, line.by, with) {
        let since = time::uptime(world);
        world.entity_mut(typing).insert(Said {
            by,
            gesture: line.gesture.clone(),
            since,
        });
    }
    let mut stage = world.resource_mut::<Stage>();
    let Some(shown) = stage.shown.as_mut() else {
        return;
    };
    let before = shown.typing.replace(typing);
    shown.typed_since = None;
    if let Some(before) = before {
        world.despawn(before);
    }
    babble_along(world, line.by, typing);
}

fn advance(world: &mut World) {
    let mut stage = world.resource_mut::<Stage>();
    let Some(shown) = stage.shown.as_mut() else {
        return;
    };
    if !shown.last_line() {
        shown.line += 1;
        shown.remark = None;
        show_line(world);
    } else if shown.choices.is_empty() {
        leave(world);
    }
}

fn press_advance(world: &mut World) {
    match typing_line(world) {
        Some(line) => {
            if let Some(mut typewriter) = world.get_mut::<Typewriter>(line) {
                typewriter.finish();
            }
        }
        None => advance(world),
    }
}

fn stage_keys(world: &mut World) {
    if world.resource::<Stage>().shown.is_none() {
        return;
    }
    let choosing = typing_line(world).is_none()
        && world
            .resource::<Stage>()
            .shown
            .as_ref()
            .is_some_and(Shown::choosing);
    if !choosing {
        if map::just_pressed(world, InputAction::Advance) {
            press_advance(world);
        }
        return;
    }
    if map::just_pressed(world, InputAction::PickChoice) {
        let selected = world
            .resource::<Stage>()
            .shown
            .as_ref()
            .map_or(0, |shown| shown.selected);
        pick_at(world, selected);
    } else if map::just_pressed(world, InputAction::ChoosePrevious) {
        step_choice(world, -1);
        InterfaceSound::UiMove.play(world);
    } else if map::just_pressed(world, InputAction::ChooseNext) {
        step_choice(world, 1);
        InterfaceSound::UiMove.play(world);
    } else if let Some(index) = map::just_pressed_index(world, &CHOICE_SHORTCUTS) {
        pick_at(world, index);
    }
}

fn step_choice(world: &mut World, delta: i32) {
    if let Some(shown) = world.resource_mut::<Stage>().shown.as_mut()
        && !shown.choices.is_empty()
    {
        let rows = shown.choices.len() as i32;
        shown.selected = (shown.selected as i32 + delta).rem_euclid(rows) as usize;
    }
}

fn pick_at(world: &mut World, index: usize) {
    let now = world.resource::<Time>().elapsed();
    let Some(shown) = world.resource_mut::<Stage>().shown.as_mut().map(|shown| {
        let locked = shown.choices.get(index).map(|choice| choice.locked);
        if locked.is_some() {
            shown.selected = index;
        }
        if locked == Some(true) {
            shown.refused = Some((index, now));
        }
        locked
    }) else {
        return;
    };
    match shown {
        Some(false) => {
            if request_pick(world, index) {
                InterfaceSound::UiPick.play(world);
            }
        }
        Some(true) => {
            request_pick(world, index);
        }
        None => {}
    }
}

fn track_typing(world: &mut World) {
    let now = world.resource::<Time>().elapsed();
    let typing = typing_line(world).is_some();
    let mut stage = world.resource_mut::<Stage>();
    let Some(shown) = stage.shown.as_mut() else {
        return;
    };
    if !typing && shown.typed_since.is_none() {
        shown.typed_since = Some(now);
    }
}

fn request_pick(world: &mut World, index: usize) -> bool {
    let mut stage = world.resource_mut::<Stage>();
    let Some(shown) = stage.shown.as_mut() else {
        return false;
    };
    if shown.picked || index >= shown.choices.len() {
        return false;
    }
    shown.picked = true;
    let step = shown.step;
    world.write_message(ConversationRequest::Pick {
        step,
        choice: index as u32,
    });
    true
}

// A locked choice was already marked refused when it was picked; one the server turned down is now.
fn refuse(world: &mut World, refused: RefusedPick) {
    let index = refused.choice as usize;
    let now = world.resource::<Time>().elapsed();
    if let Some(shown) = world.resource_mut::<Stage>().shown.as_mut()
        && shown
            .choices
            .get(index)
            .is_some_and(|choice| !choice.locked)
    {
        shown.refused = Some((index, now));
    }
}

fn babble_along(world: &mut World, by: Speaker, line: Entity) {
    let content = world.resource::<Content>().clone();
    let babble = match by {
        Speaker::Npc(npc) => npc.get(&content).voice(),
        Speaker::Player => session::me(world)
            .and_then(|seen| world.get::<Babbler>(seen))
            .map(|babbler| babbler.0),
        Speaker::Object | Speaker::Narrator => None,
    };
    if let Some(babble) = babble {
        world.entity_mut(line).insert(Babbling {
            babble,
            place: SfxPlace::Interface,
            rank: BabbleRank::Foremost,
        });
    }
}

/// The body that says a line by `by`: the player's own, or the npc's, the one talked with before
/// the nearest.
fn body_of(world: &mut World, by: Speaker, with: Option<Entity>) -> Option<Entity> {
    let npc = match by {
        Speaker::Player => return session::me(world),
        Speaker::Npc(npc) => npc,
        Speaker::Object | Speaker::Narrator => return None,
    };
    let is_npc =
        |world: &World, body: Entity| world.get::<Npc>(body).is_some_and(|seen| seen.def == npc);
    if let Some(with) = with.filter(|&with| is_npc(world, with)) {
        return Some(with);
    }
    let me = session::me(world)
        .and_then(|me| world.get::<Position>(me))
        .map(|position| position.pos);
    world
        .query::<(Entity, &Npc, &Position)>()
        .iter(world)
        .filter(|(_, seen, _)| seen.def == npc)
        .min_by(|(.., a), (.., b)| match me {
            Some(me) => a.pos.distance(me).0.total_cmp(&b.pos.distance(me).0),
            None => std::cmp::Ordering::Equal,
        })
        .map(|(body, ..)| body)
}

fn typing_line(world: &World) -> Option<Entity> {
    let line = world.resource::<Stage>().shown.as_ref()?.typing?;
    world
        .get::<Typewriter>(line)
        .is_some_and(|typewriter| !typewriter.is_finished())
        .then_some(line)
}

fn draw_stage(world: &mut World) {
    let before = std::mem::take(&mut *world.resource_mut::<StageDrawn>());
    let Some(gui) = gui::context(world) else {
        return;
    };
    let screen = gui.content_rect();
    let talking = world
        .resource::<Stage>()
        .shown
        .as_ref()
        .is_some_and(Shown::speaks);
    let dialogue = match hud::sheet_pose(world, &gui, DIALOGUE, talking) {
        Some(pose) if talking => Some(dialogue_sheet(world, &gui, screen, pose, &before)),
        Some(pose) => lift_off(world, &gui, DIALOGUE, pose),
        None => None,
    };
    let panels = docked_heads(world);
    let bottom = dialogue.map_or(screen.bottom() - SHEET_LIFT, |dialogue| {
        dialogue.top() - PANEL_GAP
    });
    match hud::sheet_pose(world, &gui, DOCKED, !panels.is_empty()) {
        Some(pose) if !panels.is_empty() => {
            docked_panels(world, &gui, panels, screen, bottom, pose)
        }
        Some(pose) => {
            lift_off(world, &gui, DOCKED, pose);
        }
        None => {}
    }
    for key in [DIALOGUE, DOCKED] {
        gui::stack(&gui, Tier::Front, sheet_layer(key));
    }
}

/// What a stage sheet last drew and where, to lift off once what it showed has gone.
#[derive(Resource, Default)]
struct StageSheets {
    dialogue_height: f32,
    drawn: HashMap<&'static str, (float::Afterimage, Rect)>,
}

fn sheet_layer(key: &'static str) -> LayerId {
    LayerId::new(Band::Stage.order(), Id::new(key))
}

fn keep_afterimage(world: &mut World, gui: &egui::Context, key: &'static str, rect: Rect) {
    let image = float::Afterimage::of(gui, sheet_layer(key));
    world
        .resource_mut::<StageSheets>()
        .drawn
        .insert(key, (image, rect));
}

fn lift_off(
    world: &World,
    gui: &egui::Context,
    key: &'static str,
    pose: SheetPose,
) -> Option<Rect> {
    let (image, rect) = world.resource::<StageSheets>().drawn.get(key)?.clone();
    gui.set_transform_layer(sheet_layer(key), pose.transform(gui, rect, rect.min));
    egui::Area::new(Id::new(key))
        .order(Band::Stage.order())
        .fixed_pos(rect.min)
        .constrain(false)
        .fade_in(false)
        .interactable(false)
        .show(gui, |ui| {
            ui.multiply_opacity(pose.opacity);
            image.paint(ui.painter());
        });
    Some(rect)
}

fn docked_heads(world: &World) -> Vec<(PanelHead, DockedPanel)> {
    world
        .get_resource::<DockedPanels>()
        .map(|docked| {
            docked
                .0
                .iter()
                .filter_map(|&panel| Some(((panel.head)(world)?, panel)))
                .collect()
        })
        .unwrap_or_default()
}

fn docked_panels(
    world: &mut World,
    gui: &egui::Context,
    panels: Vec<(PanelHead, DockedPanel)>,
    screen: Rect,
    bottom: f32,
    pose: SheetPose,
) {
    let most = (bottom - screen.top() - SCREEN_MARGIN).max(0.0);
    let id = Id::new(DOCKED);
    let rest = gui
        .memory(|memory| memory.area_rect(id))
        .unwrap_or(Rect::from_center_size(screen.center(), Vec2::ZERO));
    gui.set_transform_layer(sheet_layer(DOCKED), pose.transform(gui, rest, rest.min));
    let area = egui::Area::new(id)
        .order(Band::Stage.order())
        .pivot(Align2::CENTER_BOTTOM)
        .fixed_pos(Pos2::new(screen.center().x, bottom))
        .constrain(false)
        .fade_in(false);
    let shown = float::anchored(gui, area, |ui| {
        ui.multiply_opacity(pose.opacity);
        ui.spacing_mut().item_spacing.x = PANEL_GAP;
        ui.with_layout(Layout::left_to_right(Align::Max), |ui| {
            for (index, (head, panel)) in panels.into_iter().enumerate() {
                let width = head.width.min(screen.width() * SCREEN_SHARE);
                let rect = ui
                    .push_id(("docked", index), |ui| match head.chrome {
                        PanelChrome::Card => {
                            ui.vertical(|ui| (panel.show)(ui, world, width, most))
                                .response
                                .rect
                        }
                        PanelChrome::Sheet { title, height } => docked_sheet(
                            ui,
                            world,
                            panel,
                            &title,
                            Vec2::new(width, height.min(most)),
                            pose,
                        ),
                    })
                    .inner;
                world.resource_mut::<StageDrawn>().panels.push(rect);
            }
        });
    });
    keep_afterimage(world, gui, DOCKED, shown.response.rect);
}

fn docked_sheet(
    ui: &mut Ui,
    world: &mut World,
    panel: DockedPanel,
    title: &str,
    size: Vec2,
    pose: SheetPose,
) -> Rect {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let sheet = Sheet {
        title: Some(title),
        closable: false,
        curl: false,
        depth: sheet::Depth::Front,
    };
    let parts = sheet.paint_at(ui.painter(), rect, pose.shadow, Color32::WHITE);
    titled(ui, parts.band, title);
    let body = parts.body;
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(body)
            .layout(Layout::top_down(Align::Min)),
        |ui| (panel.show)(ui, world, body.width(), body.height()),
    );
    rect
}

fn dialogue_sheet(
    world: &mut World,
    gui: &egui::Context,
    screen: Rect,
    pose: SheetPose,
    before: &StageDrawn,
) -> Rect {
    let width = SHEET_WIDTH.min(screen.width() * SCREEN_SHARE);
    let height = world.resource::<StageSheets>().dialogue_height;
    let rect = Rect::from_min_size(
        Pos2::new(
            screen.center().x - width / 2.0,
            screen.bottom() - SHEET_LIFT - height,
        ),
        Vec2::new(width, height),
    );
    let speaker = world
        .resource::<Stage>()
        .shown
        .as_ref()
        .and_then(Shown::current)
        .and_then(|spoken| speaker_name(world, spoken.by));
    gui.set_transform_layer(layer(), pose.transform(gui, rect, rect.min));
    egui::Area::new(layer().id)
        .order(Band::Stage.order())
        .fixed_pos(rect.min)
        .constrain(false)
        .fade_in(false)
        .show(gui, |ui| {
            ui.multiply_opacity(pose.opacity);
            let (outer, held) = ui.allocate_exact_size(rect.size(), Sense::click());
            let sheet = Sheet {
                title: None,
                closable: false,
                curl: false,
                depth: sheet::Depth::Front,
            };
            let body = sheet
                .paint_at(ui.painter(), outer, pose.shadow, Color32::WHITE)
                .body;
            let written = ui
                .scope_builder(
                    UiBuilder::new()
                        .max_rect(Rect::from_min_size(
                            body.min,
                            Vec2::new(body.width(), f32::INFINITY),
                        ))
                        .layout(Layout::top_down(Align::Min)),
                    |ui| {
                        ui.spacing_mut().item_spacing.y = 12.0;
                        line(ui, world);
                        choices(ui, world, before);
                    },
                )
                .response
                .rect;
            let needed =
                written.bottom() - outer.top() + HINT_ROOM + outer.bottom() - body.bottom();
            let mut sheets = world.resource_mut::<StageSheets>();
            if (needed - sheets.dialogue_height).abs() > 0.5 {
                sheets.dialogue_height = needed;
                gui.request_discard("the conversation grew or shrank");
            }
            foot(ui, world, body);
            if let Some(speaker) = &speaker {
                name_tag(ui, outer, speaker);
            }
            if map::clicked(world, InputAction::Advance, &held) {
                press_advance(world);
            }
        });
    keep_afterimage(world, gui, DIALOGUE, rect);
    world.resource_mut::<StageDrawn>().dialogue = Some(rect);
    rect
}

fn titled(ui: &Ui, band: Rect, title: &str) {
    ui.interact(band, ui.id().with(("title", title)), Sense::hover())
        .widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, title));
}

/// The speaker's name on a slip of its own over the sheet's top edge, so it takes no room from the
/// line beneath.
fn name_tag(ui: &Ui, sheet: Rect, name: &str) {
    let galley = ui
        .painter()
        .layout_job(TextRole::Title.job(name, ink::RUBRIC));
    let padding = Vec2::new(f32::from(slip::PADDING.left), f32::from(slip::PADDING.top));
    let size = galley.size() + 2.0 * padding;
    let rect = Rect::from_min_size(
        Pos2::new(
            sheet.right() - NAME_INSET - size.x,
            sheet.top() - size.y / 2.0,
        ),
        size,
    );
    slip::paint(ui.painter(), rect);
    ui.painter().galley(rect.min + padding, galley, ink::RUBRIC);
    ui.interact(rect, ui.id().with(("speaker", name)), Sense::hover())
        .widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, name));
}

fn line(ui: &mut Ui, world: &mut World) {
    let Some(shown) = world.resource::<Stage>().shown.as_ref() else {
        return;
    };
    let Some(spoken) = shown.current() else {
        return;
    };
    let typing = shown.typing;
    let (styled, revealed) = typing
        .and_then(|line| {
            Some((
                world.get::<StyledLine>(line)?.clone(),
                world.get::<Typewriter>(line)?.shown(),
            ))
        })
        .unwrap_or_else(|| (spoken.text.styled(), None));
    egui::Frame::new().inner_margin(LINE_MARGIN).show(ui, |ui| {
        text::show(
            ui,
            world,
            &styled,
            TextLook::role(TextRole::Line, ink::INK),
            revealed,
        );
    });
}

fn choices(ui: &mut Ui, world: &mut World, before: &StageDrawn) {
    let Some(shown) = world.resource::<Stage>().shown.as_ref() else {
        return;
    };
    if !shown.choosing() {
        return;
    }
    let now = world.resource::<Time>().elapsed();
    let rows: Vec<(ChoiceView, bool)> = shown
        .choices
        .iter()
        .enumerate()
        .map(|(index, choice)| {
            let refused = shown.refused.is_some_and(|(refused, since)| {
                refused == index && now.saturating_sub(since) < REFUSED
            });
            (choice.clone(), refused)
        })
        .collect();
    let moved = ui.input(|input| input.pointer.delta() != Vec2::ZERO);
    point_at(world, pointed_choice(ui, before), moved);
    let selected = world
        .resource::<Stage>()
        .shown
        .as_ref()
        .map(|shown| shown.selected);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for (index, (choice, refused)) in rows.into_iter().enumerate() {
            let look = RowLook {
                selected: selected == Some(index),
                faint: choice.locked,
                failed: refused,
                hover: RowHover::Unlit,
            };
            let row = ui.push_id(("choice", index), |ui| {
                controls::row(ui, look, |ui| choice_row(ui, world, &choice, index))
            });
            let (label, chip_item) = row.inner.inner;
            let response = row.inner.response;
            response.widget_info(|| {
                WidgetInfo::selected(WidgetType::SelectableLabel, true, look.selected, &label)
            });
            if map::clicked(world, InputAction::PickChoice, &response) {
                pick_at(world, index);
            }
            if let Some((item, chip)) = chip_item
                && map::clicked(world, InputAction::InspectItem, &response)
            {
                card::open(world, item, Some(chip));
            }
            world
                .resource_mut::<StageDrawn>()
                .choices
                .insert(index, response.rect);
        }
    });
}

/// Found where the rows stood the pass before, as egui finds what is hovered, so the selection it
/// moves lights its row on the pass the pointer reaches it.
fn pointed_choice(ui: &Ui, before: &StageDrawn) -> Option<usize> {
    let pointer = ui.ctx().pointer_hover_pos()?;
    if ui.ctx().layer_id_at(pointer) != Some(layer()) {
        return None;
    }
    let local = ui
        .ctx()
        .layer_transform_to_global(layer())
        .map_or(pointer, |transform| transform.inverse() * pointer);
    before
        .choices
        .iter()
        .find(|(_, rect)| rect.contains(local))
        .map(|(index, _)| *index)
}

/// The pointer takes the choice only by moving onto a row, so the keys keep it from a pointer at
/// rest, even on a row that came up under it.
fn point_at(world: &mut World, pointed: Option<usize>, moved: bool) {
    let mut stage = world.resource_mut::<Stage>();
    let Some(shown) = stage.shown.as_mut() else {
        return;
    };
    if let Some(index) = pointed
        && moved
        && shown.pointed != pointed
    {
        shown.selected = index;
    }
    shown.pointed = pointed;
}

/// Hands back the item of the pill under the pointer, since the row takes every click.
fn choice_row(
    ui: &mut Ui,
    world: &mut World,
    choice: &ChoiceView,
    index: usize,
) -> (String, Option<(ItemId, Rect)>) {
    let keys = world.resource::<InputMap>();
    let shortcut = CHOICE_SHORTCUTS
        .get(index)
        .and_then(|&action| keys.name(action));
    let label = choice.label.words(keys);
    let color = ui.visuals().override_text_color.unwrap_or(ink::INK);
    let mut chip_item = None;
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            chip_item = pills(ui, world, choice);
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                if let Some(shortcut) = &shortcut {
                    controls::key_hint(ui, shortcut);
                }
                if let Some(icon) = &choice.icon
                    && let Some(image) = gui::image_file(world, icon)
                {
                    ui.add(image.widget(Vec2::splat(CHOICE_ICON)));
                }
                let look = TextLook::new(TextRole::Body.size(), color);
                text::show(ui, world, &choice.label.styled(), look, None);
            });
        });
    });
    (label, chip_item)
}

fn pills(ui: &mut Ui, world: &mut World, choice: &ChoiceView) -> Option<(ItemId, Rect)> {
    let content = world.resource::<Content>().clone();
    let mut pointed = None;
    if let Some(warn) = &choice.warn {
        controls::pill(ui, warn, Pill::Rubric);
    }
    for shown in choice.chips.iter().rev() {
        match shown {
            ChoiceChip::Needs { what, met } => {
                let (mark, pill) = if *met {
                    (Mark::Check, Pill::Verdigris)
                } else {
                    (Mark::Lock, Pill::Rubric)
                };
                controls::pill_with(ui, pill, |ui| {
                    glyph::show(ui, mark, PILL_ICON, pill.color());
                    ui.label(what.as_str());
                });
            }
            ChoiceChip::Pays { item, count, have } => {
                let pill = if have >= count {
                    Pill::Gold
                } else {
                    Pill::Rubric
                };
                let icon = item.get(&content).icon.clone();
                controls::pill_with(ui, pill, |ui| {
                    hud::icon(ui, world, &icon, PILL_ICON);
                    ui.label(count.to_string());
                });
            }
            ChoiceChip::Gets { item, count } => {
                let icon = item.get(&content).icon.clone();
                let response = controls::pill_with(ui, Pill::Verdigris, |ui| {
                    hud::icon(ui, world, &icon, PILL_ICON);
                    ui.label(format!("+{count}"));
                })
                .response
                .interact(Sense::hover());
                card::tooltip(world, &response, *item);
                if response.contains_pointer() {
                    pointed = Some((*item, response.rect));
                }
            }
        }
    }
    pointed
}

fn foot(ui: &mut Ui, world: &mut World, body: Rect) {
    let room = Rect::from_min_max(Pos2::new(body.left(), body.bottom() - HINT_ROOM), body.max);
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(room)
            .layout(Layout::left_to_right(Align::Max)),
        |ui| {
            let look = TextLook::new(TextRole::Small.size(), ink::INK3);
            text::show(ui, world, &StyledLine::new(player_hint()), look, None);
        },
    );
    let waits = world
        .resource::<Stage>()
        .shown
        .as_ref()
        .is_some_and(|shown| shown.typed_since.is_some() && !shown.choosing());
    if waits {
        let mark = Rect::from_min_max(body.max - Vec2::splat(CONTINUE_MARK), body.max);
        Mark::CaretDown.paint(ui.painter(), mark, ink::INK3);
    }
}

fn player_hint() -> Vec<TextPiece> {
    vec![
        map::input(InputAction::ChoosePrevious),
        map::input(InputAction::ChooseNext),
        TextPiece::text(" choose · "),
        map::input(InputAction::PickChoice),
        TextPiece::text(" pick · "),
        map::input(InputAction::Choice1),
        TextPiece::text("–"),
        map::input(InputAction::Choice9),
        TextPiece::text(" shortcut · "),
        map::input(InputAction::Dismiss),
        TextPiece::text(" leave"),
    ]
}

fn speaker_name(world: &World, who: Speaker) -> Option<String> {
    let content = world.resource::<Content>();
    match who {
        Speaker::Npc(npc) => Some(npc.get(content).name.to_owned()),
        Speaker::Object => world
            .resource::<Stage>()
            .shown
            .as_ref()
            .and_then(|shown| shown.with)
            .and_then(|with| world.get::<Name>(with))
            .map(|name| name.name.clone()),
        Speaker::Player => session::me(world)
            .and_then(|seen| world.get::<Name>(seen))
            .map(|name| name.name.clone()),
        Speaker::Narrator => None,
    }
}
