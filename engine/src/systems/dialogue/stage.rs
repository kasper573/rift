use std::collections::{HashMap, HashSet};
use std::time::Duration;

use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use bevy_egui::input::egui_wants_any_keyboard_input;
use design_tokens::{CHIP_RADIUS, HAIRLINE, SHADOW, TextRole, WINDOW_RADIUS, color, widgets};
use egui::{
    Align, Align2, Color32, Id, Layout, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui,
    UiBuilder, Vec2, WidgetInfo, WidgetType,
};

use super::{
    ChoiceChip, ChoiceView, Conversation, ConversationRequest, ConversationStep, RefusedPick,
    Speaker, SpokenLine,
};
use crate::core::assets::AssetRef;
use crate::core::audio::playback::SfxPlace;
use crate::core::babble::{BabbleRank, Babbler, Babbling};
use crate::core::content::{Content, Fixture};
use crate::core::gui::{self, Band};
use crate::core::text::{self, StyledLine, TextLook, TextPiece, Typewriter};
use crate::systems::actor::model::{BustFigure, Busts, Face, GenericExpression, ModelId};
use crate::systems::actor::{ActorModel, Name};
use crate::systems::hud;
use crate::systems::input::map::{self, InputAction, InputMap};
use crate::systems::interface::{InterfaceIcon, InterfaceSound};
use crate::systems::item::{ItemId, card};
use crate::systems::npc::Npc;
use crate::systems::player;
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
const BOX_WIDTH: f32 = 780.0;
const BOX_MIN_HEIGHT: f32 = 150.0;
const BOX_LIFT: f32 = 28.0;
const BOX_PADDING: Vec2 = Vec2::new(28.0, 24.0);
const PANEL_GAP: f32 = 16.0;
const SCREEN_SHARE: f32 = 0.94;
const NAME_INSET: f32 = 40.0;
const CHOICE_ICON: f32 = 18.0;
const CHIP_ICON: f32 = 14.0;
const SHAKE: Duration = Duration::from_millis(320);
const BUST_TUCK: f32 = 50.0;
const FRONT_HEIGHT: f32 = 0.64;
const BACK_HEIGHT: f32 = 0.56;
const BACK_LIFT: f32 = 0.08;
const BACK_STEP: f32 = 0.12;
const SLIDE: f32 = 70.0;
const BUST_EASE: f32 = 0.3;
const UNLIT: Color32 = Color32::from_rgb(128, 128, 143);

pub struct StagePlugin;

impl Plugin for StagePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Stage>()
            .init_resource::<StageDrawn>()
            .add_systems(OnExit(ClientScreen::Area), forget_stage)
            .add_systems(
                Update,
                (
                    follow_conversation,
                    stage_keys.run_if(not(egui_wants_any_keyboard_input)),
                    track_typing,
                )
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
    pub line: usize,
    pub lines: usize,
    pub speaker: Option<String>,
    pub text: Option<String>,
    pub typing: bool,
    pub choices: Vec<StageChoice>,
}

pub struct StageChoice {
    pub label: String,
    pub locked: bool,
    pub selected: bool,
}

#[derive(Clone, Copy)]
pub struct DockedPanel {
    pub head: fn(&World) -> Option<PanelHead>,
    pub show: fn(&mut Ui, &mut World),
}

pub struct PanelHead {
    pub title: String,
    pub width: f32,
    pub fixed_height: Option<f32>,
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
                locked: choice.locked,
                selected: index == shown.selected,
            })
            .collect(),
    })
}

pub fn dialogue_rect(world: &World) -> Option<Rect> {
    world.resource::<StageDrawn>().dialogue
}

pub fn box_right(world: &World) -> Option<f32> {
    dialogue_rect(world).map(|dialogue| dialogue.right())
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

pub fn layer() -> egui::LayerId {
    egui::LayerId::new(Band::Stage.order(), egui::Id::new("stage"))
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
    picked: bool,
    typed_since: Option<Duration>,
    faces: HashMap<Speaker, Face>,
    remark: Option<SpokenLine>,
    remarks_seen: u32,
    refusals_seen: u32,
    typing: Option<Entity>,
    selected: usize,
    pointed: Option<usize>,
    shaken: Option<(usize, Duration)>,
    cast: Vec<CastMember>,
}

#[derive(Resource, Default)]
struct StageDrawn {
    dialogue: Option<Rect>,
    panels: Vec<Rect>,
    choices: HashMap<usize, Rect>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    Left,
    Right,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Depth {
    Front,
    Back,
}

#[derive(Clone)]
struct CastMember {
    who: Speaker,
    since: Duration,
    bust: AssetRef,
    model: ModelId,
    side: Side,
    depth: Depth,
    lit: bool,
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
            stage.shown = Some(Shown::of(now, HashMap::new()));
            InterfaceSound::UiOpen.play(world);
            show_line(world);
        }
        Some(shown) if shown.step != now.step && shown.lines != now.lines => {
            let faces = std::mem::take(&mut shown.faces);
            let typing = shown.typing.take();
            stage.shown = Some(Shown::of(now, faces));
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
    fn of(now: Conversation, faces: HashMap<Speaker, Face>) -> Shown {
        Shown {
            step: now.step,
            node: now.node,
            with: now.with,
            line: 0,
            lines: now.lines,
            selected: first_open(&now.choices),
            pointed: None,
            choices: now.choices,
            picked: false,
            typed_since: None,
            faces,
            remarks_seen: now.remark.map_or(0, |remark| remark.nth),
            refusals_seen: now.refused.map_or(0, |refused| refused.nth),
            remark: None,
            typing: None,
            shaken: None,
            cast: Vec::new(),
        }
    }

    fn current(&self) -> Option<SpokenLine> {
        self.remark
            .clone()
            .or_else(|| self.lines.get(self.line).cloned())
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
    let mut stage = world.resource_mut::<Stage>();
    let Some(shown) = stage.shown.as_mut() else {
        return;
    };
    let before = shown.typing.replace(typing);
    shown.typed_since = None;
    if let Some(face) = line.face {
        shown.faces.insert(line.by, face);
    }
    if let Some(before) = before {
        world.despawn(before);
    }
    let now = world.resource::<Time>().elapsed();
    let previous_cast = world
        .resource::<Stage>()
        .shown
        .as_ref()
        .map_or_else(Vec::new, |shown| shown.cast.clone());
    let cast = cast_members(world, now, &previous_cast);
    if let Some(shown) = world.resource_mut::<Stage>().shown.as_mut() {
        shown.cast = cast;
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
    if world.resource::<Stage>().shown.is_none() || hud::confirming(world) {
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
            shown.shaken = Some((index, now));
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

// A locked choice already shook when it was picked; one the server turned down shakes now.
fn refuse(world: &mut World, refused: RefusedPick) {
    let index = refused.choice as usize;
    let now = world.resource::<Time>().elapsed();
    if let Some(shown) = world.resource_mut::<Stage>().shown.as_mut()
        && shown
            .choices
            .get(index)
            .is_some_and(|choice| !choice.locked)
    {
        shown.shaken = Some((index, now));
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

fn typing_line(world: &World) -> Option<Entity> {
    let line = world.resource::<Stage>().shown.as_ref()?.typing?;
    world
        .get::<Typewriter>(line)
        .is_some_and(|typewriter| !typewriter.is_finished())
        .then_some(line)
}

fn draw_stage(world: &mut World) {
    *world.resource_mut::<StageDrawn>() = StageDrawn::default();
    let Some(gui) = gui::context(world) else {
        return;
    };
    let panels = docked_heads(world);
    if world.resource::<Stage>().shown.is_none() && panels.is_empty() {
        return;
    }
    let screen = gui.content_rect();
    let area = egui::Area::new(Id::new("stage"))
        .order(Band::Stage.order())
        .anchor(Align2::CENTER_BOTTOM, Vec2::new(0.0, -BOX_LIFT));
    let stage = gui::show_anchored(&gui, area, |ui| {
        ui.vertical_centered(|ui| {
            if !panels.is_empty() {
                docked_panels(ui, world, panels, screen);
                ui.add_space(PANEL_GAP);
            }
            world
                .resource::<Stage>()
                .shown
                .is_some()
                .then(|| dialogue_box(ui, world, screen))
        })
        .inner
    });
    let cast = egui::Area::new(Id::new("cast"))
        .order(Band::Stage.order())
        .fixed_pos(Pos2::ZERO)
        .interactable(false)
        .show(&gui, |ui| {
            if let Some(dialogue) = stage.inner {
                busts(ui, world, dialogue, screen);
            }
        });
    let cast_layer = cast.response.layer_id;
    gui.move_to_top(cast_layer);
    gui.set_sublayer(cast_layer, stage.response.layer_id);
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
    ui: &mut Ui,
    world: &mut World,
    panels: Vec<(PanelHead, DockedPanel)>,
    screen: Rect,
) {
    let width_of = |head: &PanelHead| head.width.min(screen.width() * SCREEN_SHARE);
    let gaps = panels.len().saturating_sub(1) as f32 * ui.spacing().item_spacing.x;
    let row = panels.iter().map(|(head, _)| width_of(head)).sum::<f32>() + gaps;
    let layout = Layout::left_to_right(Align::Max);
    ui.allocate_ui_with_layout(Vec2::new(row, 0.0), layout, |ui| {
        for (index, (head, panel)) in panels.into_iter().enumerate() {
            let width = width_of(&head);
            let most = screen.height() * SCREEN_SHARE;
            let frame = egui::Frame::window(ui.style()).inner_margin(0.0);
            let shown = ui.push_id(("docked", index), |ui| {
                frame.show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.set_width(width);
                        match head.fixed_height {
                            Some(height) => ui.set_height(height.min(most)),
                            None => ui.set_max_height(most),
                        }
                        egui::Frame::new()
                            .fill(color::BG)
                            .inner_margin(egui::Margin::symmetric(10, 6))
                            .show(ui, |ui| {
                                ui.set_width(width - 20.0);
                                ui.vertical_centered(|ui| {
                                    ui.label(
                                        RichText::new(&head.title)
                                            .text_style(egui::TextStyle::Heading),
                                    );
                                });
                            });
                        (panel.show)(ui, world);
                    })
                })
            });
            world
                .resource_mut::<StageDrawn>()
                .panels
                .push(shown.response.rect);
        }
    });
}

fn dialogue_box(ui: &mut Ui, world: &mut World, screen: Rect) -> Rect {
    let width = BOX_WIDTH.min(screen.width() * SCREEN_SHARE);
    let frame = egui::Frame::new()
        .fill(color::OVERLAY)
        .stroke(HAIRLINE)
        .corner_radius(WINDOW_RADIUS)
        .shadow(SHADOW)
        .inner_margin(egui::Margin::symmetric(
            BOX_PADDING.x as i8,
            BOX_PADDING.y as i8,
        ));
    let held = ui.scope_builder(
        UiBuilder::new().id_salt("dialogue").sense(Sense::click()),
        |ui| {
            frame
                .show(ui, |ui| {
                    ui.set_width(width - 2.0 * BOX_PADDING.x);
                    ui.set_min_height(BOX_MIN_HEIGHT - 2.0 * BOX_PADDING.y);
                    ui.with_layout(Layout::top_down(Align::Min), |ui| {
                        ui.spacing_mut().item_spacing.y = 16.0;
                        line(ui, world);
                        choices(ui, world);
                    });
                })
                .response
                .rect
        },
    );
    let dialogue = held.inner;
    if map::clicked(world, InputAction::Advance, &held.response) {
        press_advance(world);
    }
    name_tag(ui, world, dialogue);
    continue_mark(ui, world, dialogue);
    hint_bar(ui, world, dialogue);
    world.resource_mut::<StageDrawn>().dialogue = Some(dialogue);
    dialogue
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
    let look = TextLook {
        size: TextRole::Line.size(),
        color: color::TEXT,
    };
    text::show(ui, world, &styled, look, revealed);
}

fn choices(ui: &mut Ui, world: &mut World) {
    let Some(shown) = world.resource::<Stage>().shown.as_ref() else {
        return;
    };
    if !shown.choosing() {
        return;
    }
    let rows: Vec<(ChoiceView, f32)> = {
        let now = world.resource::<Time>().elapsed();
        shown
            .choices
            .iter()
            .enumerate()
            .map(|(index, choice)| {
                let offset = shown
                    .shaken
                    .filter(|(shaken, _)| *shaken == index)
                    .map_or(0.0, |(_, since)| shake_offset(now.saturating_sub(since)));
                (choice.clone(), offset)
            })
            .collect()
    };
    let moved = ui.input(|input| input.pointer.delta() != Vec2::ZERO);
    let mut pointed = None;
    let mut drawn = Vec::new();
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        for (index, (choice, offset)) in rows.into_iter().enumerate() {
            let row = ui
                .push_id(("choice", index), |ui| {
                    choice_row(ui, world, &choice, index, offset)
                })
                .inner;
            if row.response.hovered() {
                pointed = Some(index);
            }
            if map::clicked(world, InputAction::PickChoice, &row.response) {
                pick_at(world, index);
            }
            if let Some(item) = row.chip_item
                && map::clicked(world, InputAction::InspectItem, &row.response)
            {
                card::open(world, item);
            }
            world
                .resource_mut::<StageDrawn>()
                .choices
                .insert(index, row.response.rect);
            drawn.push(row);
        }
    });
    point_at(world, pointed, moved);
    let selected = world
        .resource::<Stage>()
        .shown
        .as_ref()
        .map(|shown| shown.selected);
    for (index, row) in drawn.iter().enumerate() {
        highlight(ui, row, selected == Some(index));
    }
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

fn shake_offset(age: Duration) -> f32 {
    if age >= SHAKE {
        return 0.0;
    }
    let fade = 1.0 - age.as_secs_f32() / SHAKE.as_secs_f32();
    (age.as_secs_f32() * 70.0).sin() * 6.0 * fade
}

struct ChoiceRow {
    response: egui::Response,
    label: String,
    background: egui::layers::ShapeIdx,
    chip_item: Option<ItemId>,
}

fn choice_row(
    ui: &mut Ui,
    world: &mut World,
    choice: &ChoiceView,
    index: usize,
    offset: f32,
) -> ChoiceRow {
    let keys = world.resource::<InputMap>();
    let shortcut = CHOICE_SHORTCUTS
        .get(index)
        .and_then(|&action| keys.name(action));
    let label = choice.label.words(keys);
    let ink = if choice.locked {
        color::TEXT.gamma_multiply(0.45)
    } else {
        color::TEXT
    };
    let room = Rect::from_min_size(
        ui.cursor().min + Vec2::new(offset, 0.0),
        Vec2::new(ui.available_width(), 0.0),
    );
    let background = ui.painter().add(egui::Shape::Noop);
    let mut chip_item = None;
    let row = ui.scope_builder(
        UiBuilder::new()
            .max_rect(room.with_max_y(ui.max_rect().max.y))
            .layout(Layout::top_down(Align::Min))
            .sense(Sense::click()),
        |ui| {
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(8, 2))
                .show(ui, |ui| {
                    ui.set_width(room.width() - 16.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            chip_item = chips(ui, world, choice);
                            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                if let Some(shortcut) = &shortcut {
                                    widgets::key_hint(ui, shortcut);
                                }
                                if let Some(icon) = &choice.icon
                                    && let Some(image) = gui::image_file(world, icon)
                                {
                                    ui.add(image.widget(Vec2::splat(CHOICE_ICON)));
                                }
                                let look = TextLook {
                                    size: TextRole::Line.size(),
                                    color: ink,
                                };
                                text::show(ui, world, &choice.label.styled(), look, None);
                            });
                        });
                    });
                });
        },
    );
    ChoiceRow {
        response: row.response,
        label,
        background,
        chip_item,
    }
}

/// Painted once the pass has moved the selection, so a row lights up on the frame the pointer
/// reaches it.
fn highlight(ui: &Ui, row: &ChoiceRow, selected: bool) {
    row.response.widget_info(|| {
        WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, &row.label)
    });
    if !selected {
        return;
    }
    let visuals = ui.style().interact_selectable(&row.response, selected);
    ui.painter().set(
        row.background,
        egui::epaint::RectShape::new(
            row.response.rect,
            4.0,
            visuals.weak_bg_fill,
            visuals.bg_stroke,
            StrokeKind::Inside,
        ),
    );
}

/// The row takes every click, so it is handed the item of the chip under the pointer to inspect.
fn chips(ui: &mut Ui, world: &mut World, choice: &ChoiceView) -> Option<ItemId> {
    let content = world.resource::<Content>().clone();
    let mut pointed = None;
    if let Some(warn) = &choice.warn {
        chip(ui, world, None, warn, color::DANGER);
    }
    for shown in choice.chips.iter().rev() {
        match shown {
            ChoiceChip::Needs { what, met } => {
                let (icon, ink) = if *met {
                    (InterfaceIcon::Checked, color::OK)
                } else {
                    (InterfaceIcon::Locked, color::DANGER)
                };
                let icon = icon.get(&content).image.clone();
                chip(ui, world, Some(&icon), what, ink);
            }
            ChoiceChip::Pays { item, count, have } => {
                let ink = if have >= count {
                    color::WARN
                } else {
                    color::DANGER
                };
                let icon = item.get(&content).icon.clone();
                chip(ui, world, Some(&icon), &count.to_string(), ink);
            }
            ChoiceChip::Gets { item, count } => {
                let icon = item.get(&content).icon.clone();
                let response = chip(ui, world, Some(&icon), &format!("+{count}"), color::OK);
                card::tooltip(world, &response, *item);
                if response.contains_pointer() {
                    pointed = Some(*item);
                }
            }
        }
    }
    pointed
}

fn chip(
    ui: &mut Ui,
    world: &mut World,
    icon: Option<&AssetRef>,
    label: &str,
    ink: Color32,
) -> egui::Response {
    egui::Frame::new()
        .fill(ink.gamma_multiply(0.12))
        .stroke(Stroke::new(1.0, ink.gamma_multiply(0.6)))
        .corner_radius(CHIP_RADIUS)
        .inner_margin(egui::Margin::symmetric(6, 1))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                if let Some(icon) = icon {
                    hud::icon(ui, world, icon, CHIP_ICON);
                }
                ui.label(RichText::new(label).size(12.5).color(ink));
            });
        })
        .response
        .interact(Sense::hover())
}

fn name_tag(ui: &mut Ui, world: &World, dialogue: Rect) {
    let Some(spoken) = world
        .resource::<Stage>()
        .shown
        .as_ref()
        .and_then(Shown::current)
    else {
        return;
    };
    let Some(name) = speaker_name(world, spoken.by) else {
        return;
    };
    let text = RichText::new(name).text_style(egui::TextStyle::Heading);
    let galley = egui::WidgetText::from(text.clone()).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Heading,
    );
    let size = galley.size() + Vec2::new(24.0, 8.0);
    let x = match side_of(spoken.by) {
        Side::Left => dialogue.left() + NAME_INSET,
        Side::Right => dialogue.right() - NAME_INSET - size.x,
    };
    let rect = Rect::from_min_size(Pos2::new(x, dialogue.top() - size.y / 2.0), size);
    ui.put(rect, |ui: &mut Ui| {
        egui::Frame::new()
            .fill(color::RAISED)
            .stroke(HAIRLINE)
            .corner_radius(4.0)
            .inner_margin(egui::Margin::symmetric(12, 4))
            .show(ui, |ui| ui.add(egui::Label::new(text).extend()))
            .response
    });
}

fn continue_mark(ui: &Ui, world: &World, dialogue: Rect) {
    let waits = world
        .resource::<Stage>()
        .shown
        .as_ref()
        .is_some_and(|shown| shown.typed_since.is_some() && !shown.choosing());
    if waits {
        ui.painter().text(
            dialogue.right_bottom() - Vec2::new(BOX_PADDING.x, 12.0),
            Align2::RIGHT_BOTTOM,
            "↓",
            egui::FontId::proportional(12.0),
            color::MUTED,
        );
    }
}

fn hint_bar(ui: &mut Ui, world: &mut World, dialogue: Rect) {
    let hint = player_hint();
    let room = Rect::from_min_size(
        dialogue.left_bottom() + Vec2::new(16.0, -12.0),
        Vec2::new(dialogue.width() - 32.0, 24.0),
    );
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(room)
            .layout(Layout::left_to_right(Align::Min)),
        |ui| {
            egui::Frame::new()
                .fill(color::INPUT)
                .stroke(HAIRLINE)
                .corner_radius(4.0)
                .inner_margin(egui::Margin::symmetric(8, 3))
                .show(ui, |ui| {
                    let look = TextLook {
                        size: TextRole::Small.size(),
                        color: color::MUTED,
                    };
                    text::show(ui, world, &StyledLine::new(hint), look, None);
                });
        },
    );
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

fn busts(ui: &mut Ui, world: &mut World, dialogue: Rect, screen: Rect) {
    let Some(cast) = world
        .resource::<Stage>()
        .shown
        .as_ref()
        .map(|shown| shown.cast.clone())
    else {
        return;
    };
    let content = world.resource::<Content>().clone();
    let now = world.resource::<Time>().elapsed();
    let gui = ui.ctx().clone();
    for depth in [Depth::Back, Depth::Front] {
        for member in cast.iter().filter(|member| member.depth == depth) {
            let Some(image) = gui::image(world, &member.bust) else {
                continue;
            };
            let figure = ActorModel::get(&content, member.model)
                .bust_figure()
                .unwrap_or(BustFigure {
                    left: 0.0,
                    right: 1.0,
                });
            let (share, lift) = match member.depth {
                Depth::Front => (FRONT_HEIGHT, 0.0),
                Depth::Back => (BACK_HEIGHT, BACK_LIFT),
            };
            let height = screen.height() * share;
            let size = Vec2::new(height * image.size.x / image.size.y.max(1.0), height);
            let rect = bust_rect(member, figure, size, dialogue, screen, lift);
            let entered = (now.saturating_sub(member.since).as_secs_f32() / BUST_EASE).min(1.0);
            let shown = egui::emath::easing::cubic_out(entered);
            let lit =
                gui.animate_bool_with_time(Id::new(("lit", member.who)), member.lit, BUST_EASE);
            let away = match member.side {
                Side::Left => -1.0,
                Side::Right => 1.0,
            };
            let rect = rect.translate(Vec2::new(away * SLIDE * (1.0 - shown), 0.0));
            let tint = lerp_color(UNLIT, Color32::WHITE, lit).gamma_multiply(shown);
            ui.put(rect, image.widget(size).tint(tint));
        }
    }
}

fn bust_rect(
    member: &CastMember,
    figure: BustFigure,
    size: Vec2,
    dialogue: Rect,
    screen: Rect,
    lift: f32,
) -> Rect {
    let figure_width = (figure.right - figure.left) * size.x;
    let step = match member.depth {
        Depth::Front => 0.0,
        Depth::Back => screen.width() * BACK_STEP,
    };
    let left = match member.side {
        Side::Right => {
            let near = (dialogue.right() - BUST_TUCK).min(screen.right() - figure_width) - step;
            near - figure.left * size.x
        }
        Side::Left => {
            let near = (dialogue.left() + BUST_TUCK).max(screen.left() + figure_width) + step;
            near - figure.right * size.x
        }
    };
    let bottom = screen.bottom() - screen.height() * lift;
    Rect::from_min_size(Pos2::new(left, bottom - size.y), size)
}

fn lerp_color(from: Color32, to: Color32, t: f32) -> Color32 {
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    Color32::from_rgb(
        mix(from.r(), to.r()),
        mix(from.g(), to.g()),
        mix(from.b(), to.b()),
    )
}

fn cast_members(world: &World, now: Duration, before: &[CastMember]) -> Vec<CastMember> {
    let Some(shown) = world.resource::<Stage>().shown.as_ref() else {
        return Vec::new();
    };
    let Some(current) = shown.current() else {
        return Vec::new();
    };
    let mut speakers = vec![current.by];
    if shown.remark.is_none() {
        speakers.push(Speaker::Player);
        if let Some(npc) = shown.with.and_then(|with| world.get::<Npc>(with)) {
            speakers.push(Speaker::Npc(npc.def));
        }
        for line in shown.lines.iter().take(shown.line + 1) {
            speakers.push(line.by);
        }
    }
    let mut seen = HashSet::new();
    speakers.retain(|who| seen.insert(*who));
    let content = world.resource::<Content>();
    speakers.retain(|&who| model_of(content, who).is_some());
    let fronts: Vec<Speaker> = [Side::Left, Side::Right]
        .into_iter()
        .filter_map(|side| {
            let mut standing = speakers.iter().filter(|&&who| side_of(who) == side);
            standing
                .clone()
                .find(|&&who| who == current.by)
                .or_else(|| standing.next())
                .copied()
        })
        .collect();
    speakers
        .into_iter()
        .filter_map(|who| {
            let model = model_of(content, who)?;
            let busts = busts_of(content, model)?;
            let face = shown
                .faces
                .get(&who)
                .cloned()
                .unwrap_or(Face::Generic(GenericExpression::Neutral));
            let bust = busts
                .face(&face)
                .unwrap_or_else(|| busts.generic.of(GenericExpression::Neutral));
            Some(CastMember {
                who,
                since: before
                    .iter()
                    .find(|member| member.who == who)
                    .map_or(now, |member| member.since),
                bust: bust.clone(),
                model,
                side: side_of(who),
                depth: if fronts.contains(&who) {
                    Depth::Front
                } else {
                    Depth::Back
                },
                lit: who == current.by,
            })
        })
        .collect()
}

fn model_of(content: &Content, who: Speaker) -> Option<ModelId> {
    let model = match who {
        Speaker::Npc(npc) => npc.get(content).model,
        Speaker::Player => player::def(content).model,
        Speaker::Object | Speaker::Narrator => return None,
    };
    busts_of(content, model).map(|_| model)
}

fn busts_of(content: &Content, model: ModelId) -> Option<&Busts> {
    model.get(content).busts()
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

fn side_of(who: Speaker) -> Side {
    match who {
        Speaker::Player => Side::Left,
        Speaker::Npc(_) | Speaker::Object | Speaker::Narrator => Side::Right,
    }
}
