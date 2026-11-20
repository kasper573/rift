//! A letter being written: who it is for, its title and text, and the stacks of the bag placed
//! among its attachments. It is this client's alone until it is sent, and survives the menu
//! closing.

use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ui::egui::{self, Align, Layout, Sense, Ui};
use ui::input::InputField;
use ui::{controls, form, ink, sheet};

use super::{LetterAnswered, SendLetter};
use crate::core::content::Content;
use crate::systems::GameSettings;
use crate::systems::interface::InterfaceSound;
use crate::systems::item::bag_view::{BagView, bag_view};
use crate::systems::item::transfer::{self, ItemTransfer, TransferPane, TrayWindow, tray_window};
use crate::systems::item::tray::ItemTray;
use crate::systems::item::widget::slot_note_source;
use crate::systems::item::{self, ItemStack};
use crate::systems::mail::MailMessages;
use crate::systems::messages;
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;
use crate::systems::settings::Placed;
use crate::systems::society::tab::{self, CharacterAction};
use crate::systems::society::{AccountNumber, SocietyMember, SocietyView};

/// Writing to a player a page lists.
pub struct WriteToCharacter;

impl CharacterAction for WriteToCharacter {
    fn label<'a>(&self, messages: &'a messages::Messages) -> &'a str {
        messages.mail.write()
    }

    fn on(&self, world: &World) -> bool {
        world.resource::<GameSettings>().mail.is_some()
    }

    fn act(&self, world: &mut World, player: &SocietyMember) {
        write_to(world, Some(player.account));
        tab::open_on(world, super::page::KEY);
    }
}

/// The letter the player is writing, for a driver of the client.
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

/// Begins a letter to `to`, or goes on with the one being written when it is for them already
/// or for nobody yet.
pub fn write_to(world: &mut World, to: Option<AccountNumber>) {
    let mut draft = world.resource_mut::<Draft>();
    draft.writing = true;
    if to.is_some() {
        draft.to = to;
    }
}

/// Whether the page shows the letter being written.
pub(super) fn writing(world: &World) -> bool {
    world.resource::<Draft>().writing
}

pub(super) fn plugin(app: &mut App) {
    tray_window::<LetterAttachments>(app);
    slot_note_source(app, note);
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
pub(crate) fn may_write_to(view: &SocietyView, player: AccountNumber) -> bool {
    receivers(view).iter().any(|other| other.account == player)
}

/// The players a letter can be written to: the character's friends, and the members of their
/// party.
fn receivers(view: &SocietyView) -> Vec<SocietyMember> {
    let friends = view.friends.iter().map(|friend| &friend.character);
    let party = view.party.iter().flat_map(|party| &party.members);
    let mut receivers: Vec<SocietyMember> = Vec::new();
    for player in friends.chain(party.map(|member| &member.character)) {
        let listed = receivers
            .iter()
            .any(|other| other.account == player.account);
        if player.account != view.account && !listed {
            receivers.push(player.clone());
        }
    }
    receivers
}

/// The page while a letter is being written: the letter on the left, the bag to attach from
/// on the right.
pub(super) fn show(ui: &mut Ui, world: &mut World, view: &SocietyView) {
    let Some(rules) = world.resource::<GameSettings>().mail else {
        return;
    };
    let content = world.resource::<Content>().clone();
    let messages = messages::of(&content);
    let room = ui.available_rect_before_wrap();
    let letter_width = room.width() - BAG_WIDTH - 4.0 * sheet::PANE_PAD.x;
    let ([letter_at], bag_at) = sheet::columns(ui, room, [letter_width]);
    let area = egui::ScrollArea::vertical().id_salt("letter being written");
    sheet::scroll_pane(ui, letter_at, area, |ui| {
        letter(ui, world, &messages.mail, view, &rules);
    });
    let bag = egui::UiBuilder::new()
        .max_rect(bag_at.inner)
        .layout(egui::Layout::top_down(egui::Align::Min));
    ui.scope_builder(bag, |ui| {
        let view = BagView {
            heading: messages.interface.inventory(),
            counts: true,
            placement: Placed::Menu,
        };
        bag_view(ui, world, view);
    });
    ui.allocate_rect(room, Sense::hover());
}

fn letter(
    ui: &mut Ui,
    world: &mut World,
    words: &MailMessages,
    view: &SocietyView,
    rules: &super::MailSettings,
) {
    let receivers = receivers(view);
    let mut back = false;
    sheet::section_with(ui, words.new_letter(), |ui| {
        back = controls::button(ui, words.back_to_inbox()).clicked();
    });
    let mut draft = world.resource_mut::<Draft>();
    if draft
        .to
        .is_some_and(|to| !receivers.iter().any(|player| player.account == to))
    {
        draft.to = None;
    }
    ui.add_enabled_ui(!draft.sending, |ui| {
        let to = ui.label(words.to());
        let names: Vec<String> = receivers.iter().map(|player| player.name.clone()).collect();
        let selected = receivers
            .iter()
            .position(|player| Some(player.account) == draft.to);
        let search = controls::SearchWords {
            hint: if receivers.is_empty() {
                words.no_receivers()
            } else {
                words.choose_receiver()
            },
            none_found: words.no_receiver_found(),
        };
        let drawn = controls::search_combo(
            ui,
            "letter to",
            &controls::NamedChoices(&names),
            selected,
            search,
            Some(RECEIVER_WIDTH),
        );
        drawn.response.labelled_by(to.id);
        if let Some(player) = drawn.picked.and_then(|place| receivers.get(place)) {
            draft.to = Some(player.account);
        }
        if receivers.is_empty() {
            caption(ui, words.receivers_note());
        }
        ui.add_space(GAP);
        let title = ui.label(words.title_label());
        let width = ui.available_width();
        InputField::new(&mut draft.title)
            .char_limit(rules.title_length as usize)
            .width(width)
            .show(ui)
            .labelled_by(title.id);
        let written = draft.title.chars().count();
        caption(ui, &words.written_of(written, rules.title_length));
        ui.add_space(GAP);
        let text = ui.label(words.text_label());
        InputField::new(&mut draft.text)
            .rows(TEXT_ROWS)
            .char_limit(rules.text_length as usize)
            .width(width)
            .show(ui)
            .labelled_by(text.id);
        let written = draft.text.chars().count();
        caption(ui, &words.written_of(written, rules.text_length));
    });
    ui.add_space(GAP);
    let (tray, sending) = (draft.tray.clone(), draft.sending);
    let slots = (tray.placed().len() as u32 + 1).min(rules.attachments);
    let attached = words.attached_of(tray.placed().len(), rules.attachments);
    form::section_count(ui, words.attached(), attached);
    ui.add_enabled_ui(!sending, |ui| {
        let pane = TransferPane {
            placed: tray.placed(),
            slots,
            named: words.attached(),
            transfer: Some(ItemTransfer::of_tray::<LetterAttachments>()),
        };
        transfer::transfer_pane(ui, world, pane);
    });
    ui.add_space(GAP);
    let draft = world.resource::<Draft>();
    let ready = draft.to.is_some() && !draft.title.trim().is_empty();
    let mut sent = false;
    let mut discarded = false;
    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
        sent = if draft.sending {
            controls::waiting(ui, words.send());
            false
        } else if ready {
            controls::primary(ui, words.send()).clicked()
        } else {
            let needs = match draft.to {
                None => words.choose_receiver(),
                Some(_) => words.needs_a_title(),
            };
            controls::faded(ui, words.send()).on_hover_text(needs);
            false
        };
        discarded = ui
            .add_enabled_ui(!draft.sending, |ui| {
                controls::button(ui, words.discard()).clicked()
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
    let letter = SendLetter {
        to,
        title: draft.title.trim().to_owned(),
        text: draft.text.clone(),
        attached: draft.tray.covered(),
    };
    world.write_message(letter);
    InterfaceSound::UiPick.play(world);
}

/// The letter being written as a window items move into: its attachments, while the page shows it.
struct LetterAttachments;

impl TrayWindow for LetterAttachments {
    fn open(world: &World) -> bool {
        writing(world) && tab::showing(world, super::page::KEY)
    }

    fn tray(world: &World) -> &ItemTray {
        &world.resource::<Draft>().tray
    }

    fn tray_mut(world: &mut World) -> Option<&mut ItemTray> {
        let draft = world.resource_mut::<Draft>().into_inner();
        (!draft.sending).then_some(&mut draft.tray)
    }

    fn slots(world: &World) -> usize {
        world
            .resource::<GameSettings>()
            .mail
            .map_or(0, |rules| rules.attachments as usize)
    }
}

fn note(world: &World, stack: ItemStack) -> Option<String> {
    let messages = messages::of(world.resource::<Content>());
    world
        .resource::<Draft>()
        .tray
        .holds(stack.item)
        .then(|| messages.mail.attached_slot().to_owned())
}

/// Keeps what is attached within what the bag holds, and hears whether the letter was sent:
/// one that was is done with, and one that was refused is the player's to change and send again.
fn settle(world: &mut World) {
    let answered = world
        .resource_mut::<Messages<LetterAnswered>>()
        .drain()
        .last();
    let bag = item::my_bag(world).cloned();
    let mut draft = world.resource_mut::<Draft>();
    match answered {
        Some(LetterAnswered { sent: true }) => *draft = Draft::default(),
        Some(LetterAnswered { sent: false }) => draft.sending = false,
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
