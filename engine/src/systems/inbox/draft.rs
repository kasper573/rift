//! A message being written: who it is for, its title and text, and the stacks of the bag placed
//! among its attachments. It is this client's alone until it is sent, and survives the menu
//! closing.

use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ui::egui::{self, Align, Layout, Sense, Ui};
use ui::input::InputField;
use ui::{controls, form, ink, sheet};

use super::{MessageAnswered, SendMessage};
use crate::core::content::Content;
use crate::systems::GameSettings;
use crate::systems::input::map::InputAction;
use crate::systems::interface::InterfaceSound;
use crate::systems::item::tray::{ItemTray, TrayDone, tray_slots};
use crate::systems::item::widget::{
    InventoryPane, SlotAction, SlotVerdict, inventory_pane, slot_action, slot_aside_source,
    slot_note_source, slot_verdict_source,
};
use crate::systems::item::{Inventory, ItemStack};
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;
use crate::systems::society::tab::{self, PlayerAction};
use crate::systems::society::{AccountNumber, SocietyMember, SocietyView};

/// Writing to a player a page lists.
pub struct WriteToPlayer;

impl PlayerAction for WriteToPlayer {
    fn label(&self) -> &'static str {
        "Write a message"
    }

    fn on(&self, world: &World) -> bool {
        world.resource::<GameSettings>().messages.is_some()
    }

    fn act(&self, world: &mut World, player: &SocietyMember) {
        write_to(world, Some(player.account));
        tab::open_on(world, super::page::TITLE);
    }
}

/// The message the player is writing, for a driver of the client.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ShownDraft {
    /// Who it is for, once chosen.
    pub to: Option<String>,
    pub title: String,
    pub text: String,
    /// Each stack attached, as its count and its item's name.
    pub attached: Vec<String>,
}

pub fn shown(world: &World) -> Option<ShownDraft> {
    let draft = world.resource::<Draft>();
    let view = session::my_character(world)?.get::<SocietyView>()?;
    let content = world.resource::<Content>();
    draft.writing.then(|| ShownDraft {
        to: receivers(view)
            .into_iter()
            .find(|player| Some(player.account) == draft.to)
            .map(|player| player.name),
        title: draft.title.clone(),
        text: draft.text.clone(),
        attached: draft
            .tray
            .placed()
            .iter()
            .map(|placed| placed.stack.describe(content))
            .collect(),
    })
}

/// Begins a message to `to`, or goes on with the one being written when it is for them already
/// or for nobody yet.
pub fn write_to(world: &mut World, to: Option<AccountNumber>) {
    let mut draft = world.resource_mut::<Draft>();
    draft.writing = true;
    if to.is_some() {
        draft.to = to;
    }
}

/// Whether the page shows the message being written.
pub(super) fn writing(world: &World) -> bool {
    world.resource::<Draft>().writing
}

pub(super) fn plugin(app: &mut App) {
    slot_action(
        app,
        SlotAction {
            action: InputAction::PlaceItem,
            active: attaching,
            act: |world, slot| attach(world, slot, false),
            verb: "attach",
        },
    );
    slot_action(
        app,
        SlotAction {
            action: InputAction::PlaceStack,
            active: attaching,
            act: |world, slot| attach(world, slot, true),
            verb: "attach the stack",
        },
    );
    slot_aside_source(app, aside);
    slot_note_source(app, note);
    slot_verdict_source(app, verdict);
    app.init_resource::<Draft>()
        .add_systems(Update, settle.run_if(in_state(ClientScreen::Area)))
        .add_systems(OnExit(ClientScreen::Area), forget);
}

#[derive(Resource, Default)]
struct Draft {
    writing: bool,
    to: Option<AccountNumber>,
    title: String,
    text: String,
    tray: ItemTray,
    /// Whether it was sent and the server has yet to answer.
    sending: bool,
}

/// Whether the player `view` is of can write to `player`.
pub(super) fn may_write_to(view: &SocietyView, player: AccountNumber) -> bool {
    receivers(view).iter().any(|other| other.account == player)
}

/// The players a message can be written to: the player's friends, and the members of their
/// party.
fn receivers(view: &SocietyView) -> Vec<SocietyMember> {
    let friends = view.friends.iter().map(|friend| &friend.player);
    let party = view.party.iter().flat_map(|party| &party.members);
    let mut receivers: Vec<SocietyMember> = Vec::new();
    for player in friends.chain(party.map(|member| &member.player)) {
        let listed = receivers
            .iter()
            .any(|other| other.account == player.account);
        if player.account != view.account && !listed {
            receivers.push(player.clone());
        }
    }
    receivers
}

/// The page while a message is being written: the message on the left, the bag to attach from
/// on the right.
pub(super) fn show(ui: &mut Ui, world: &mut World, view: &SocietyView) {
    let Some(rules) = world.resource::<GameSettings>().messages else {
        return;
    };
    let room = ui.available_rect_before_wrap();
    let message_width = room.width() - BAG_WIDTH - 4.0 * sheet::PANE_PAD.x;
    let ([message_at], bag_at) = sheet::columns(ui, room, [message_width]);
    let area = egui::ScrollArea::vertical().id_salt("message being written");
    sheet::scroll_pane(ui, message_at, area, |ui| {
        message(ui, world, view, &rules);
    });
    let area = egui::ScrollArea::vertical().id_salt("bag to attach from");
    sheet::scroll_pane(ui, bag_at, area, |ui| {
        let pane = InventoryPane {
            width: ui.available_width(),
            heading: Some("Your inventory"),
        };
        inventory_pane(ui, world, pane);
    });
    ui.allocate_rect(room, Sense::hover());
}

fn message(ui: &mut Ui, world: &mut World, view: &SocietyView, rules: &super::MessageSettings) {
    let receivers = receivers(view);
    let mut back = false;
    sheet::section_with(ui, "A new message", |ui| {
        back = controls::button(ui, "Back to the inbox").clicked();
    });
    let mut draft = world.resource_mut::<Draft>();
    if draft
        .to
        .is_some_and(|to| !receivers.iter().any(|player| player.account == to))
    {
        draft.to = None;
    }
    ui.add_enabled_ui(!draft.sending, |ui| {
        let to = ui.label("To");
        let chosen = receivers
            .iter()
            .find(|player| Some(player.account) == draft.to);
        let shown = match (chosen, receivers.is_empty()) {
            (Some(player), _) => player.name.as_str(),
            (None, true) => "No friends or party members yet",
            (None, false) => "Choose who it is for",
        };
        controls::combo("message to")
            .selected_text(shown)
            .width(RECEIVER_WIDTH)
            .show_ui(ui, |ui| {
                for player in &receivers {
                    ui.selectable_value(&mut draft.to, Some(player.account), &player.name);
                }
            })
            .response
            .labelled_by(to.id);
        if receivers.is_empty() {
            caption(ui, "Messages go to friends and party members.");
        }
        ui.add_space(GAP);
        let title = ui.label("Title");
        let width = ui.available_width();
        InputField::new(&mut draft.title)
            .char_limit(rules.title_length as usize)
            .width(width)
            .show(ui)
            .labelled_by(title.id);
        let written = draft.title.chars().count();
        caption(ui, &format!("{written} of {}", rules.title_length));
        ui.add_space(GAP);
        let text = ui.label("Message");
        InputField::new(&mut draft.text)
            .rows(TEXT_ROWS)
            .char_limit(rules.text_length as usize)
            .width(width)
            .show(ui)
            .labelled_by(text.id);
        let written = draft.text.chars().count();
        caption(
            ui,
            &format!("{written} of {} characters", rules.text_length),
        );
    });
    ui.add_space(GAP);
    let (tray, sending) = (draft.tray.clone(), draft.sending);
    let slots = (tray.placed().len() as u32 + 1).min(rules.attachments);
    let attached = format!("{} of {}", tray.placed().len(), rules.attachments);
    form::section_count(ui, "Attached", attached);
    let done = ui
        .add_enabled_ui(!sending, |ui| {
            tray_slots(ui, world, &tray, slots, "Attached")
        })
        .inner;
    match done {
        Some(TrayDone::Clicked(index)) => {
            world.resource_mut::<Draft>().tray.take_back(index);
            InterfaceSound::UiMove.play(world);
        }
        Some(TrayDone::Dropped(slot)) => attach(world, slot, true),
        None => {}
    }
    form::caption(ui, "Click an attached item to take it back", ink::INK3);
    ui.add_space(GAP);
    let draft = world.resource::<Draft>();
    let ready = draft.to.is_some() && !draft.title.trim().is_empty();
    let mut sent = false;
    let mut discarded = false;
    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
        sent = if draft.sending {
            controls::waiting(ui, "Send");
            false
        } else if ready {
            controls::primary(ui, "Send").clicked()
        } else {
            let needs = match draft.to {
                None => "Choose who it is for",
                Some(_) => "Give it a title",
            };
            controls::faded(ui, "Send").on_hover_text(needs);
            false
        };
        discarded = ui
            .add_enabled_ui(!draft.sending, |ui| {
                controls::button(ui, "Discard").clicked()
            })
            .inner;
    });
    if sent {
        send(world);
    } else if discarded {
        *world.resource_mut::<Draft>() = Draft::default();
        InterfaceSound::UiClose.play(world);
    } else if back {
        world.resource_mut::<Draft>().writing = false;
    }
}

fn send(world: &mut World) {
    let mut draft = world.resource_mut::<Draft>();
    let Some(to) = draft.to else {
        return;
    };
    draft.sending = true;
    let message = SendMessage {
        to,
        title: draft.title.trim().to_owned(),
        text: draft.text.clone(),
        attached: draft.tray.covered(),
    };
    world.write_message(message);
    InterfaceSound::UiPick.play(world);
}

/// Whether the bag's slots attach to the message being written: while the page shows it.
fn attaching(world: &World) -> bool {
    writing(world) && tab::showing(world, super::page::TITLE)
}

fn attach(world: &mut World, slot: u32, whole: bool) {
    let most = world
        .resource::<GameSettings>()
        .messages
        .map_or(0, |rules| rules.attachments as usize);
    let content = world.resource::<Content>().clone();
    let bag = session::my_character(world).and_then(|me| me.get::<Inventory>().cloned());
    let stays = |stack: &ItemStack| stack.item.get(&content).bound();
    let attached = match bag {
        Some(bag) if !bag.slots.get(slot as usize).is_some_and(stays) => {
            let mut draft = world.resource_mut::<Draft>();
            !draft.sending && draft.tray.place(&bag, slot, whole, most)
        }
        _ => false,
    };
    if attached {
        InterfaceSound::UiMove.play(world);
    } else {
        InterfaceSound::UiRefuse.play(world);
    }
}

fn aside(world: &World, slot: u32, stack: ItemStack) -> u32 {
    let tray = &world.resource::<Draft>().tray;
    session::my_character(world)
        .and_then(|me| me.get::<Inventory>())
        .map_or(0, |bag| tray.aside(bag, slot, stack))
}

fn note(world: &World, stack: ItemStack) -> Option<String> {
    world
        .resource::<Draft>()
        .tray
        .holds(stack.item)
        .then(|| "attached · click it in the message to take it back".to_owned())
}

/// An item that may not change hands is refused where it lies, while a message is written.
fn verdict(world: &World, stack: ItemStack) -> Option<SlotVerdict> {
    let stays = stack.item.get(world.resource::<Content>()).bound();
    (stays && attaching(world)).then(|| SlotVerdict::Refused("It stays with you".into()))
}

/// Keeps what is attached within what the bag holds, and hears whether the message was sent:
/// one that was is done with, and one that was refused is the player's to change and send again.
fn settle(world: &mut World) {
    let answered = world
        .resource_mut::<Messages<MessageAnswered>>()
        .drain()
        .last();
    let bag = session::my_character(world).and_then(|me| me.get::<Inventory>().cloned());
    let mut draft = world.resource_mut::<Draft>();
    match answered {
        Some(MessageAnswered { sent: true }) => *draft = Draft::default(),
        Some(MessageAnswered { sent: false }) => draft.sending = false,
        None => {}
    }
    if draft.tray.is_empty() {
        return;
    }
    if let Some(bag) = bag {
        let mut tray = draft.tray.clone();
        tray.keep_within(&bag);
        tray.drop_short();
        if tray != draft.tray {
            draft.tray = tray;
        }
    }
}

fn forget(mut draft: ResMut<Draft>) {
    *draft = Draft::default();
}

fn caption(ui: &mut Ui, text: &str) {
    form::caption(ui, text, ink::INK2);
}

const BAG_WIDTH: f32 = 290.0;
const RECEIVER_WIDTH: f32 = 260.0;
const TEXT_ROWS: usize = 6;
const GAP: f32 = 8.0;
