//! A prompt as its player sees it: a panel docked where a shop's counter docks, with the script's
//! words, a tray of slots that say nothing of what belongs in them, and one button. What is
//! placed in the tray stays in the bag and is known to this client alone until it is submitted.

use bevy::prelude::*;
use ui::controls;
use ui::drag;
use ui::egui::{self, Sense, Ui, Vec2};
use ui::face::TextRole;
use ui::{ink, sheet};

use super::{PromptKind, PromptRequest, PromptStep, PromptView};
use crate::core::content::Content;
use crate::core::text::{self, StyledLine, TextLook, TextPiece};
use crate::systems::GameSettings;
use crate::systems::dialogue::Conversation;
use crate::systems::dialogue::stage::{self, DockedPanel, PanelChrome, PanelHead};
use crate::systems::hud::{self, Carried};
use crate::systems::input::map::{InputAction, InputMap, input};
use crate::systems::interface::InterfaceSound;
use crate::systems::item::widget::{
    SLOT, SlotAction, SlotLook, slot, slot_action, slot_aside_source, slot_note_source,
};
use crate::systems::item::{Inventory, ItemId, ItemStack};
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;

const SLOT_GAP: f32 = 6.0;
pub const TRAY_WIDTH: f32 = 320.0;
const PADDING: f32 = 12.0;
const LINE: f32 = 22.0;
const BUTTON_GAP: f32 = 10.0;

pub struct PromptTrayPlugin;

impl Plugin for PromptTrayPlugin {
    fn build(&self, app: &mut App) {
        slot_action(
            app,
            SlotAction {
                action: InputAction::PlaceItem,
                active: open,
                act: |world, slot| place(world, slot, false),
                verb: "place",
            },
        );
        slot_action(
            app,
            SlotAction {
                action: InputAction::PlaceStack,
                active: open,
                act: |world, slot| place(world, slot, true),
                verb: "place the stack",
            },
        );
        slot_aside_source(app, aside);
        slot_note_source(app, note);
        stage::dock(app, DockedPanel { head, show });
        app.init_resource::<Placing>()
            .add_systems(Update, settle.run_if(in_state(ClientScreen::Area)))
            .add_systems(OnExit(ClientScreen::Area), forget);
    }
}

/// What the player has placed in the tray of the prompt before them, slot by slot.
#[derive(Resource, Default)]
pub struct Placing {
    placed: Vec<Placed>,
    /// The turn the placed items were submitted for, until the server answers it.
    sent: Option<PromptStep>,
    seen: Option<PromptStep>,
    height: f32,
}

/// A stack placed in a slot of the tray; `short` once the bag no longer covers what was placed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placed {
    pub stack: ItemStack,
    pub short: bool,
}

/// The prompt before the player, if one is open.
pub fn view(world: &World) -> Option<&PromptView> {
    session::me(world).and_then(|me| world.get::<PromptView>(me))
}

/// What is placed in the tray, slot by slot.
pub fn placed(world: &World) -> &[Placed] {
    &world.resource::<Placing>().placed
}

/// What submitting the tray offers: every stack placed that the bag still covers.
pub fn offer(world: &World) -> Vec<ItemStack> {
    placed(world)
        .iter()
        .filter(|placed| !placed.short)
        .map(|placed| placed.stack)
        .collect()
}

/// Whether what is placed was submitted and the server has yet to answer.
pub fn waiting(world: &World) -> bool {
    let placing = world.resource::<Placing>();
    placing.sent.is_some() && placing.sent == view(world).map(|view| view.step)
}

/// Marks what is placed as submitted for the turn `step`, so a second press sends nothing.
pub fn sent(world: &mut World, step: PromptStep) {
    world.resource_mut::<Placing>().sent = Some(step);
}

/// Places one of the item in the bag's slot `slot` in the tray, or with `whole` all of that slot
/// that is not placed yet: on the stack of it already there, else in the next empty slot.
pub fn place(world: &mut World, slot: u32, whole: bool) {
    let Some(slots) = view(world).map(|view| view.slots as usize) else {
        return;
    };
    let Some(bag) = session::my_character(world).and_then(|me| me.get::<Inventory>()) else {
        return;
    };
    let Some(&stack) = bag.slots.get(slot as usize) else {
        return;
    };
    let placing = world.resource::<Placing>();
    let already: u32 = placing
        .placed
        .iter()
        .filter(|placed| placed.stack.item == stack.item)
        .map(|placed| placed.stack.count)
        .sum();
    let free = bag.count(stack.item).saturating_sub(already);
    let count = if whole {
        stack.count.min(free)
    } else {
        free.min(1)
    };
    let room = placing.placed.len() < slots
        || placing
            .placed
            .iter()
            .any(|placed| placed.stack.item == stack.item);
    if count == 0 || !room {
        InterfaceSound::UiRefuse.play(world);
        return;
    }
    put(world, ItemStack::new(stack.item, count));
    InterfaceSound::UiMove.play(world);
}

/// Places `stack` in the tray whatever is there: on the stack of its item, else in a slot of
/// its own.
pub fn put(world: &mut World, stack: ItemStack) {
    let mut placing = world.resource_mut::<Placing>();
    match placing
        .placed
        .iter_mut()
        .find(|placed| placed.stack.item == stack.item)
    {
        Some(placed) => placed.stack.count += stack.count,
        None => placing.placed.push(Placed {
            stack,
            short: false,
        }),
    }
}

/// Takes back what the tray's slot `index` holds.
pub fn take_back(world: &mut World, index: usize) {
    let mut placing = world.resource_mut::<Placing>();
    if index < placing.placed.len() {
        placing.placed.remove(index);
    }
}

/// Empties the tray.
pub fn clear(world: &mut World) {
    world.resource_mut::<Placing>().placed.clear();
}

/// The tray's slots: every placed stack in its slot, a click on one taking it back, and what is
/// dropped on them placed whole.
pub fn slots(ui: &mut Ui, world: &mut World, count: u32) {
    let content = world.resource::<Content>().clone();
    let held: Vec<Placed> = placed(world).to_vec();
    let mut taken = None;
    let shown = ui
        .scope(|ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(SLOT_GAP);
            ui.horizontal_wrapped(|ui| {
                for index in 0..count as usize {
                    let Some(placed) = held.get(index) else {
                        slot(ui, world, SlotLook::EMPTY, Sense::hover()).widget_info(|| {
                            egui::WidgetInfo::labeled(egui::WidgetType::Label, true, "Empty slot")
                        });
                        continue;
                    };
                    let def = placed.stack.item.get(&content);
                    let look = SlotLook {
                        icon: Some(&def.icon),
                        count: placed.stack.count,
                        refused: placed.short,
                        ..SlotLook::EMPTY
                    };
                    let response = slot(ui, world, look, Sense::click());
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Button,
                            true,
                            format!("Placed: {} {}", placed.stack.count, def.name),
                        )
                    });
                    if response.clicked() {
                        taken = Some(index);
                    }
                    let lines = if placed.short {
                        vec!["You no longer carry this".to_owned()]
                    } else {
                        vec!["Click to take it back".to_owned()]
                    };
                    hud::tooltip(&response, world, &def.name, &lines, None);
                }
            });
        })
        .response;
    if let Some(index) = taken {
        take_back(world, index);
        InterfaceSound::UiMove.play(world);
    }
    if let Some(Carried::BagSlot(slot)) = drag::dropped::<Carried>(&shown) {
        place(world, slot, true);
    }
}

/// How tall a tray of `slots` slots stands in a panel `width` wide.
pub fn slots_height(slots: u32, width: f32) -> f32 {
    let across = (((width + SLOT_GAP) / (SLOT + SLOT_GAP)).floor() as u32).max(1);
    let rows = slots.div_ceil(across).max(1) as f32;
    rows * SLOT + (rows - 1.0) * SLOT_GAP
}

/// The line of the script's own words over a prompt's tray, when it has one.
pub fn words(ui: &mut Ui, world: &World, view: &PromptView) {
    if let Some(text) = &view.text {
        ui.label(
            egui::RichText::new(text.words(world.resource::<InputMap>()))
                .size(TextRole::Body.size())
                .color(ink::INK2),
        );
        ui.add_space(6.0);
    }
}

/// The foot of a prompt: how its tray is filled and left, and the button that submits what is
/// placed, faded while nothing is or the server has yet to answer.
pub fn foot(ui: &mut Ui, world: &mut World, submit: impl FnOnce(&mut World)) {
    let room = Vec2::new(ui.available_width(), controls::CONTROL_HEIGHT);
    let across = egui::Layout::right_to_left(egui::Align::Center);
    ui.allocate_ui_with_layout(room, across, |ui| {
        let word = world
            .resource::<Content>()
            .built::<GameSettings>()
            .words
            .confirm
            .clone();
        let ready = !offer(world).is_empty() && !waiting(world);
        if !ready {
            controls::faded(ui, word);
        } else if controls::primary(ui, word).clicked() {
            submit(world);
            InterfaceSound::UiPick.play(world);
        }
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            place_hint(ui, world);
        });
    });
}

/// How tall the prompt's sheet stands: as tall as what it last drew, `guessed` before it has.
pub fn fitted(world: &World, guessed: f32) -> f32 {
    let measured = world.resource::<Placing>().height;
    if measured > 0.0 { measured } else { guessed }
}

/// Keeps the prompt's sheet as tall as what it drew, `drawn` high, turned out to be.
pub fn fit(ui: &Ui, world: &mut World, drawn: f32) {
    let needed = drawn + sheet::chrome(true).y;
    let mut placing = world.resource_mut::<Placing>();
    if (needed - placing.height).abs() > 0.5 {
        placing.height = needed;
        ui.ctx().request_discard("the prompt grew or shrank");
    }
}

/// How a prompt's tray is filled and left, for whoever has not done it before.
pub fn place_hint(ui: &mut Ui, world: &mut World) {
    let hint = StyledLine::new(vec![
        input(InputAction::PlaceItem),
        TextPiece::text(" or "),
        input(InputAction::CarryItem),
        TextPiece::text(" to place · "),
        input(InputAction::Dismiss),
        TextPiece::text(" leave"),
    ]);
    let look = TextLook::new(TextRole::Small.size(), ink::INK3);
    text::show(ui, world, &hint, look, None);
}

fn open(world: &World) -> bool {
    view(world).is_some()
}

fn head(world: &World) -> Option<PanelHead> {
    let view = view(world).filter(|view| view.kind == PromptKind::Offer)?;
    let chrome = sheet::chrome(true);
    let text = if view.text.is_some() { LINE + 6.0 } else { 0.0 };
    let guessed = chrome.y
        + 2.0 * PADDING
        + text
        + slots_height(view.slots, TRAY_WIDTH - chrome.x - 2.0 * PADDING)
        + BUTTON_GAP
        + controls::CONTROL_HEIGHT;
    Some(PanelHead {
        width: TRAY_WIDTH,
        chrome: PanelChrome::Sheet {
            title: view.title.clone(),
            height: fitted(world, guessed),
        },
    })
}

fn show(ui: &mut Ui, world: &mut World, _width: f32, _most: f32) {
    let Some(view) = view(world).cloned() else {
        return;
    };
    let drawn = egui::Frame::new()
        .inner_margin(PADDING)
        .show(ui, |ui| {
            words(ui, world, &view);
            slots(ui, world, view.slots);
            ui.add_space(BUTTON_GAP);
            foot(ui, world, |world| {
                world.write_message(PromptRequest::Submit {
                    step: view.step,
                    offer: offer(world),
                });
                sent(world, view.step);
            });
        })
        .response
        .rect;
    fit(ui, world, drawn.height());
}

/// How many of the stack in the bag's slot `slot` are placed in the tray: what is placed of an
/// item is counted off its stacks from the last one back, as the bag gives them up.
fn aside(world: &World, slot: u32, stack: ItemStack) -> u32 {
    let placed: u32 = placed(world)
        .iter()
        .filter(|placed| placed.stack.item == stack.item)
        .map(|placed| placed.stack.count)
        .sum();
    if placed == 0 {
        return 0;
    }
    let later: u32 = session::my_character(world)
        .and_then(|me| me.get::<Inventory>())
        .map_or(0, |bag| {
            bag.slots
                .iter()
                .skip(slot as usize + 1)
                .filter(|other| other.item == stack.item)
                .map(|other| other.count)
                .sum()
        });
    placed.saturating_sub(later).min(stack.count)
}

fn note(world: &World, stack: ItemStack) -> Option<String> {
    placed(world)
        .iter()
        .any(|placed| placed.stack.item == stack.item)
        .then(|| "placed · click it in the prompt to take it back".to_owned())
}

/// Keeps what is placed within what the bag holds, marking what it no longer covers, and empties
/// the tray once the conversation that held the prompt is over.
fn settle(world: &mut World) {
    let me = session::me(world);
    let step = view(world).map(|view| view.step);
    let talking = me.is_some_and(|me| world.get::<Conversation>(me).is_some());
    let held: Vec<(ItemId, u32)> = me
        .and_then(|me| world.get::<Inventory>(me))
        .map(|bag| {
            placed(world)
                .iter()
                .map(|placed| (placed.stack.item, bag.count(placed.stack.item)))
                .collect()
        })
        .unwrap_or_default();
    let mut placing = world.resource_mut::<Placing>();
    if step.is_none() && !talking {
        if !placing.placed.is_empty() || placing.sent.is_some() {
            *placing = Placing::default();
        }
        return;
    }
    let fresh = step.is_some() && placing.seen != step;
    let opened = fresh && placing.seen.is_none();
    if placing.seen != step {
        placing.seen = step;
    }
    let mut left: Vec<(ItemId, u32)> = held;
    for placed in &mut placing.placed {
        let have = left
            .iter_mut()
            .find(|(item, _)| *item == placed.stack.item)
            .map_or(0, |(_, have)| std::mem::take(have));
        if have < placed.stack.count {
            placed.stack.count = have;
            placed.short = true;
        }
    }
    if fresh {
        placing.placed.retain(|placed| placed.stack.count > 0);
        for placed in &mut placing.placed {
            placed.short = false;
        }
    }
    if opened {
        InterfaceSound::UiOpen.play(world);
    }
}

fn forget(mut placing: ResMut<Placing>) {
    *placing = Placing::default();
}
