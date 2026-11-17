//! A prompt as its player sees it: a sheet docked where a shop's counter docks, with the script's
//! words, a tray of slots that say nothing of what belongs in them, and the inventory to fill
//! them from. What is placed in the tray stays in the bag and is known to this client alone until
//! it is submitted.

use bevy::prelude::*;
use ui::controls;
use ui::egui::{self, Align, Layout, Pos2, Rect, Ui, UiBuilder, Vec2};
use ui::face::TextRole;
use ui::{ink, sheet};

use super::{PromptKind, PromptRequest, PromptStep, PromptView};
use crate::core::content::Content;
use crate::core::text::{self, StyledLine, TextLook};
use crate::systems::GameSettings;
use crate::systems::dialogue::Conversation;
use crate::systems::dialogue::stage::{self, DockedPanel, PanelChrome, PanelHead};
use crate::systems::input::map::InputMap;
use crate::systems::interface::InterfaceSound;
use crate::systems::item::transfer::{self, ItemTransfer, TransferPane, item_transfer};
pub use crate::systems::item::transfer::{slots_height, slots_width};
use crate::systems::item::tray::ItemTray;
pub use crate::systems::item::tray::Placed;
use crate::systems::item::widget::{InventoryPane, inventory_pane, slot_note_source};
use crate::systems::item::{Inventory, ItemStack};
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;

const SHEET: Vec2 = Vec2::new(680.0, 320.0);
pub const INVENTORY_WIDTH: f32 = 290.0;
const BUTTON_GAP: f32 = 10.0;

pub struct PromptTrayPlugin;

impl Plugin for PromptTrayPlugin {
    fn build(&self, app: &mut App) {
        item_transfer(app, TRANSFER);
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
    tray: ItemTray,
    /// The turn the placed items were submitted for, until the server answers it.
    sent: Option<PromptStep>,
    seen: Option<PromptStep>,
}

/// The prompt before the player, if one is open.
pub fn view(world: &World) -> Option<&PromptView> {
    session::me(world).and_then(|me| world.get::<PromptView>(me))
}

/// What is placed in the tray, slot by slot.
pub fn placed(world: &World) -> &[Placed] {
    world.resource::<Placing>().tray.placed()
}

/// What submitting the tray offers: every stack placed that the bag still covers.
pub fn offer(world: &World) -> Vec<ItemStack> {
    world.resource::<Placing>().tray.covered()
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

/// The tray as a window items move into: what was submitted stays as it is until the server
/// answers.
const TRANSFER: ItemTransfer = ItemTransfer {
    key: "prompt",
    open,
    verb: "place",
    put: place,
    take: take_back,
    aside,
};

/// Places `count` of the item in the bag's slot `slot` in the tray: on the stack of it already
/// there, else in the next empty slot.
fn place(world: &mut World, slot: u32, count: u32) -> bool {
    let Some(slots) = view(world).map(|view| view.slots as usize) else {
        return false;
    };
    let bag = session::my_character(world).and_then(|me| me.get::<Inventory>().cloned());
    match bag {
        Some(bag) if !waiting(world) => {
            let tray = &mut world.resource_mut::<Placing>().tray;
            tray.place(&bag, slot, count, slots)
        }
        _ => false,
    }
}

/// Places `stack` in the tray whatever is there: on the stack of its item, else in a slot of
/// its own.
pub fn put(world: &mut World, stack: ItemStack) {
    world.resource_mut::<Placing>().tray.put(stack);
}

/// Takes back `count` of what the tray's slot `index` holds.
fn take_back(world: &mut World, index: usize, count: u32) -> bool {
    !waiting(world) && world.resource_mut::<Placing>().tray.take_back(index, count)
}

/// Empties the tray.
pub fn clear(world: &mut World) {
    world.resource_mut::<Placing>().tray.clear();
}

/// The tray's slots: every placed stack in its slot.
pub fn slots(ui: &mut Ui, world: &mut World, count: u32) {
    let tray = world.resource::<Placing>().tray.clone();
    let pane = TransferPane {
        placed: tray.placed(),
        slots: count,
        named: "Placed",
        transfer: Some(TRANSFER),
    };
    transfer::transfer_pane(ui, world, pane);
}

/// How the tray is filled and emptied, in a line.
pub fn how_to() -> StyledLine {
    transfer::how_to(&TRANSFER)
}

/// The line of the script's own words over a prompt's tray, when it has one.
pub fn words(ui: &mut Ui, world: &World, view: &PromptView) {
    if let Some(text) = &view.text {
        ui.label(
            egui::RichText::new(
                text.words(world.resource::<Content>(), world.resource::<InputMap>()),
            )
            .size(TextRole::Body.size())
            .color(ink::INK2),
        );
        ui.add_space(6.0);
    }
}

pub fn heading(ui: &mut Ui, world: &World, view: &PromptView) {
    sheet::section(ui, "Placed");
    words(ui, world, view);
}

pub fn inventory(ui: &mut Ui, world: &mut World, locked: bool) {
    lockable(ui, locked, |ui| {
        let room = ui.available_rect_before_wrap();
        sheet::scroll_pane(
            ui,
            sheet::Pane::bare(room),
            egui::ScrollArea::vertical(),
            |ui| {
                let pane = InventoryPane {
                    width: ui.available_width(),
                    heading: Some("Your inventory"),
                };
                inventory_pane(ui, world, pane);
            },
        );
    });
}

pub fn lockable(ui: &mut Ui, locked: bool, add: impl FnOnce(&mut Ui)) {
    ui.add_enabled_ui(!locked, add);
}

pub fn leave_button(ui: &mut Ui, world: &mut World) {
    if controls::button(ui, "Leave").clicked() {
        stage::leave(world);
    }
}

fn open(world: &World) -> bool {
    view(world).is_some()
}

fn head(world: &World) -> Option<PanelHead> {
    let view = view(world).filter(|view| view.kind == PromptKind::Offer)?;
    Some(PanelHead {
        width: SHEET.x,
        chrome: PanelChrome::Sheet {
            title: view.title.clone(),
            height: SHEET.y,
        },
    })
}

fn show(ui: &mut Ui, world: &mut World, _width: f32, _most: f32) {
    let Some(view) = view(world).cloned() else {
        return;
    };
    egui::Frame::new()
        .inner_margin(sheet::PAGE_MARGIN)
        .show(ui, |ui| {
            let room = ui.available_rect_before_wrap();
            let inventory_at = Rect::from_min_max(
                Pos2::new(room.max.x - INVENTORY_WIDTH, room.min.y),
                room.max,
            );
            let tray = Rect::from_min_max(
                room.min,
                Pos2::new(inventory_at.min.x - sheet::GUTTER, room.max.y),
            );
            let column = |rect| {
                UiBuilder::new()
                    .max_rect(rect)
                    .layout(Layout::top_down(Align::Min))
            };
            ui.scope_builder(column(tray), |ui| {
                heading(ui, world, &view);
                slots(ui, world, view.slots);
                ui.with_layout(Layout::bottom_up(Align::Min), |ui| foot(ui, world, &view));
            });
            ui.scope_builder(column(inventory_at), |ui| inventory(ui, world, false));
            ui.advance_cursor_after_rect(room);
        });
}

fn foot(ui: &mut Ui, world: &mut World, view: &PromptView) {
    let room = Vec2::new(ui.available_width(), controls::CONTROL_HEIGHT);
    ui.allocate_ui_with_layout(room, Layout::right_to_left(Align::Center), |ui| {
        let word = world
            .resource::<Content>()
            .built::<GameSettings>()
            .words
            .confirm
            .clone();
        if waiting(world) {
            controls::waiting(ui, word);
        } else if offer(world).is_empty() {
            controls::faded(ui, word);
        } else if controls::primary(ui, word).clicked() {
            world.write_message(PromptRequest::Submit {
                step: view.step,
                offer: offer(world),
            });
            sent(world, view.step);
            InterfaceSound::UiPick.play(world);
        }
        leave_button(ui, world);
    });
    ui.add_space(BUTTON_GAP);
    let hint = how_to();
    let look = TextLook::new(TextRole::Small.size(), ink::INK3);
    text::show(ui, world, &hint, look, None);
}

/// How many of the stack in the bag's slot `slot` are placed in the tray.
fn aside(world: &World, slot: u32, stack: ItemStack) -> u32 {
    let tray = &world.resource::<Placing>().tray;
    session::my_character(world)
        .and_then(|me| me.get::<Inventory>())
        .map_or(0, |bag| tray.aside(bag, slot, stack))
}

fn note(world: &World, stack: ItemStack) -> Option<String> {
    world
        .resource::<Placing>()
        .tray
        .holds(stack.item)
        .then(|| "placed · click it in the prompt to take it back".to_owned())
}

/// Keeps what is placed within what the bag holds, marking what it no longer covers, and empties
/// the tray once the conversation that held the prompt is over.
fn settle(world: &mut World) {
    let me = session::me(world);
    let step = view(world).map(|view| view.step);
    let talking = me.is_some_and(|me| world.get::<Conversation>(me).is_some());
    let bag = me.and_then(|me| world.get::<Inventory>(me)).cloned();
    let mut placing = world.resource_mut::<Placing>();
    if step.is_none() && !talking {
        if !placing.tray.is_empty() || placing.sent.is_some() {
            *placing = Placing::default();
        }
        return;
    }
    let fresh = step.is_some() && placing.seen != step;
    let opened = fresh && placing.seen.is_none();
    if placing.seen != step {
        placing.seen = step;
    }
    if let Some(bag) = &bag {
        placing.tray.keep_within(bag);
    }
    if fresh {
        placing.tray.drop_short();
    }
    if opened {
        InterfaceSound::UiOpen.play(world);
    }
}

fn forget(mut placing: ResMut<Placing>) {
    *placing = Placing::default();
}
