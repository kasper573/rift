use std::collections::HashMap;
use std::time::Duration;
use ui::egui;
use ui::float;

use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use ui::egui::{LayerId, Rect, Vec2};
use ui::motion::SheetPose;

use super::sheet::{self, DialogueSheet, SheetChoice, SheetToCome};
use super::{
    ChoiceChip, ChoiceView, Conversation, ConversationRequest, RefusedPick, Speaker, SpokenLine,
    WindowStep, windows,
};
use crate::core::audio::playback::SfxPlace;
use crate::core::babble::{BabbleRank, Babbler, Babbling};
use crate::core::content::Content;
use crate::core::gui::{self, Band, Tier};
use crate::core::script::DialogueCamera;
use crate::core::text::Typewriter;
use crate::core::time::{self};
use crate::core::units::MeterPos;
use crate::systems::actor::Name;
use crate::systems::actor::render::Said;
use crate::systems::hud::{self, WindowKey};
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
const REFUSED: Duration = Duration::from_millis(320);
const DIALOGUE: WindowKey = WindowKey::Stage;

pub struct DialogueStagePlugin;

impl Plugin for DialogueStagePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Stage>()
            .init_resource::<StageDrawn>()
            .init_resource::<StageSheets>()
            .init_resource::<windows::Seating>()
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
        text: shown.current().map(|line| {
            line.text.words(
                world.resource::<Content>(),
                &world.resource::<InputMap>().key_names(),
            )
        }),
        typing: shown.typed_since.is_none(),
        choices: shown
            .choices
            .iter()
            .enumerate()
            .map(|(index, choice)| StageChoice {
                label: choice.label.words(
                    world.resource::<Content>(),
                    &world.resource::<InputMap>().key_names(),
                ),
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

/// Where all the dialogue shows stands while it is on show: every line of its step and the choices
/// that end it, as they will stand from the step's first letter.
pub fn dialogue_rect(world: &World) -> Option<Rect> {
    world.resource::<StageDrawn>().dialogue
}

/// Where the dialogue and the windows it opened stand, which a camera that frames the
/// conversation leaves room for.
pub fn rects(world: &World) -> Vec<Rect> {
    let drawn = world.resource::<gui::WidgetRects>();
    dialogue_rect(world)
        .into_iter()
        .chain(
            WindowKey::OF_A_CONVERSATION
                .iter()
                .filter_map(|key| drawn.0.get(key.name()).copied()),
        )
        .collect()
}

pub fn choice_rects(world: &World) -> HashMap<usize, Rect> {
    world.resource::<StageDrawn>().choices.clone()
}

pub fn leavable(world: &World) -> bool {
    world.resource::<Stage>().shown.is_some()
}

pub fn layer() -> LayerId {
    sheet::layer()
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
struct Stage {
    shown: Option<Shown>,
}

struct Shown {
    step: WindowStep,
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
    let styled = line.text.styled(world.resource::<Content>());
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
        Speaker::Character => session::me(world)
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
        Speaker::Character => return session::me(world),
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
    windows::begin_pass(world);
    let before = std::mem::take(&mut *world.resource_mut::<StageDrawn>());
    let Some(gui) = gui::context(world) else {
        return;
    };
    let talking = world
        .resource::<Stage>()
        .shown
        .as_ref()
        .is_some_and(Shown::speaks);
    match hud::sheet_pose(world, &gui, DIALOGUE, talking) {
        Some(pose) if talking => dialogue_text(world, &gui, pose, &before),
        Some(pose) => lift_off(world, &gui, pose),
        None => {}
    }
    gui::stack(&gui, Tier::Front, layer());
}

/// What the dialogue last drew and how tall it stood, to lift off once what it showed has gone.
#[derive(Resource, Default)]
struct StageSheets {
    dialogue_height: f32,
    drawn: Option<(float::Afterimage, Rect)>,
}

fn keep_afterimage(world: &mut World, gui: &egui::Context, rect: Rect) {
    let image = float::Afterimage::of(gui, layer());
    world.resource_mut::<StageSheets>().drawn = Some((image, rect));
}

fn lift_off(world: &World, gui: &egui::Context, pose: SheetPose) {
    let Some((image, rect)) = world.resource::<StageSheets>().drawn.clone() else {
        return;
    };
    gui.set_transform_layer(layer(), pose.transform(gui, rect, rect.min));
    egui::Area::new(layer().id)
        .order(Band::Stage.order())
        .fixed_pos(rect.min)
        .constrain(false)
        .fade_in(false)
        .interactable(false)
        .show(gui, |ui| {
            ui.multiply_opacity(pose.opacity);
            image.paint(ui.painter());
        });
}

/// The conversation as free text over the world, as the sheet draws it, and what the player does
/// with it.
fn dialogue_text(world: &mut World, gui: &egui::Context, pose: SheetPose, before: &StageDrawn) {
    let pointed = pointed_choice(gui, before);
    let moved = gui.input(|input| input.pointer.delta() != Vec2::ZERO);
    point_at(world, pointed, moved);
    let Some(shown) = sheet_of(world) else {
        return;
    };
    let height = world.resource::<StageSheets>().dialogue_height;
    let drawn = sheet::draw(world, gui, &shown, pose, height);
    world.resource_mut::<StageSheets>().dialogue_height = drawn.height;
    if map::clicked(world, InputAction::Advance, &drawn.held) {
        press_advance(world);
    }
    for (index, row) in drawn.rows.iter().enumerate() {
        if map::clicked(world, InputAction::PickChoice, &row.response) {
            pick_at(world, index);
        }
        if let Some((item, chip)) = row.pointed_item
            && map::clicked(world, InputAction::InspectItem, &row.response)
        {
            card::open(world, item, Some(chip));
        }
        let pointed: Vec<ItemId> = row.pointed_item.iter().map(|(item, _)| *item).collect();
        card::chat_shortcut(world, &row.response, &pointed);
        world
            .resource_mut::<StageDrawn>()
            .choices
            .insert(index, row.response.rect);
    }
    keep_afterimage(world, gui, drawn.rect);
    world.resource_mut::<StageDrawn>().dialogue = Some(drawn.content);
}

/// What the conversation on show puts on the sheet: its line as far as it has typed, and its
/// choices once the last line is out, with room kept for every line of its step and the choices
/// that follow them.
fn sheet_of(world: &World) -> Option<DialogueSheet> {
    let shown = world.resource::<Stage>().shown.as_ref()?;
    let spoken = shown.current();
    let typed = shown
        .typing
        .and_then(|line| world.get::<Typewriter>(line))
        .and_then(Typewriter::shown);
    let choosing = shown.choosing();
    let now = world.resource::<Time>().elapsed();
    let choices = if choosing {
        shown
            .choices
            .iter()
            .enumerate()
            .map(|(index, choice)| SheetChoice {
                view: choice.clone(),
                lit: index == shown.selected,
                refused: shown.refused.is_some_and(|(refused, since)| {
                    refused == index && now.saturating_sub(since) < REFUSED
                }),
            })
            .collect()
    } else {
        Vec::new()
    };
    Some(DialogueSheet {
        speaker: spoken
            .as_ref()
            .and_then(|line| speaker_name(world, line.by)),
        line: spoken.map(|line| line.text),
        typed,
        waits: shown.typed_since.is_some() && !choosing,
        choices,
        to_come: SheetToCome {
            lines: shown
                .lines
                .iter()
                .map(|line| (speaker_name(world, line.by), line.text.clone()))
                .collect(),
            choices: shown.choices.clone(),
        },
    })
}

/// Found where the rows stood the pass before, as egui finds what is hovered, so the selection it
/// moves lights its row on the pass the pointer reaches it.
fn pointed_choice(gui: &egui::Context, before: &StageDrawn) -> Option<usize> {
    let pointer = gui.pointer_hover_pos()?;
    if gui.layer_id_at(pointer) != Some(layer()) {
        return None;
    }
    let local = gui
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
        Speaker::Character => session::me(world)
            .and_then(|seen| world.get::<Name>(seen))
            .map(|name| name.name.clone()),
        Speaker::Narrator => None,
    }
}
