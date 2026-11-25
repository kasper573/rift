//! The Mail page of the Social tab: the inbox, the letter chosen in it with what is
//! attached, and the letter being written.

use super::inbox;
use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ui::egui::{self, Align, Layout, Sense, Ui};
use ui::{controls, face, form, ink, sheet};

use super::draft::{self, ShownDraft};
use super::{
    DeleteLetter, InboxRow, InboxSender, InboxView, LetterId, LetterTexts, MailMessages,
    OpenLetter, TakeAttached, TakenStack,
};
use crate::core::content::Content;
use crate::core::time::LocalClock;
use crate::systems::GameSettings;
use crate::systems::hud::{self, Confirm};
use crate::systems::input::map::{self, InputAction};
use crate::systems::item::transfer::{
    self, ItemTransfer, TransferPane, TransferWindow, transfer_window,
};
use crate::systems::item::tray::Placed;
use crate::systems::messages;
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;
use crate::systems::society::SocietyView;
use crate::systems::society::tab::SocialPage;

pub(crate) const KEY: &str = "mail";

pub struct MailPage;

impl SocialPage for MailPage {
    fn key(&self) -> &'static str {
        KEY
    }

    fn title<'a>(&self, messages: &'a messages::Messages) -> &'a str {
        messages.mail.title()
    }

    fn on(&self, world: &World) -> bool {
        world.resource::<GameSettings>().mail.is_some()
    }

    fn waiting(&self, world: &World) -> u32 {
        inbox(world).map_or(0, |inbox| inbox.unread())
    }

    fn hints(&self, world: &World) -> Vec<(InputAction, String)> {
        let messages = messages::of(world.resource::<Content>());
        if draft::writing(world) {
            return transfer::moves(&messages.item).to_vec();
        }
        let attached = inbox(world)
            .and_then(|inbox| chosen(inbox, world.resource::<Reading>()))
            .is_some_and(|row| !row.attached.is_empty());
        let moves = transfer::moves(&messages.item)
            .into_iter()
            .filter(|_| attached);
        let listed = inbox(world).is_some_and(|inbox| inbox.whole && !inbox.rows.is_empty());
        listed
            .then(|| (InputAction::Select, messages.mail.hint_choose().to_owned()))
            .into_iter()
            .chain(moves)
            .collect()
    }

    fn show(&self, ui: &mut Ui, world: &mut World) {
        let me = session::my_character(world);
        let Some(society) = me.and_then(|me| me.get::<SocietyView>().cloned()) else {
            return;
        };
        if draft::writing(world) {
            draft::show(ui, world, &society);
            return;
        }
        let content = world.resource::<Content>().clone();
        let messages = &messages::of(&content).mail;
        let inbox = inbox(world).cloned().unwrap_or_default();
        let room = ui.available_rect_before_wrap();
        let ([list_at], reading_at) = sheet::columns(ui, room, [LIST_WIDTH]);
        let mut done = None;
        let area = egui::ScrollArea::vertical().id_salt("inbox");
        sheet::scroll_pane(ui, list_at, area, |ui| {
            done = done.take().or(list(ui, world, messages, &inbox));
        });
        let area = egui::ScrollArea::vertical().id_salt("letter read");
        sheet::scroll_pane(ui, reading_at, area, |ui| {
            done = done
                .take()
                .or(reading(ui, world, messages, &inbox, &society));
        });
        ui.allocate_rect(room, Sense::hover());
        if let Some(done) = done {
            done.apply(world);
        }
    }
}

/// What the Mail page shows beside the inbox, for a driver of the client.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
pub struct ShownMailPage {
    /// The letter chosen in the inbox.
    pub open: Option<LetterId>,
    /// The letter being written.
    pub draft: Option<ShownDraft>,
}

pub fn shown(world: &World) -> ShownMailPage {
    ShownMailPage {
        open: inbox(world)
            .and_then(|inbox| chosen(inbox, world.resource::<Reading>()))
            .map(|row| row.id),
        draft: draft::shown(world),
    }
}

pub(crate) fn plugin(app: &mut App) {
    draft::plugin(app);
    transfer_window::<ReceivedAttachments>(app);
    app.init_resource::<Reading>()
        .add_systems(OnExit(ClientScreen::Area), forget);
}

/// The letter the player chose to read.
#[derive(Resource, Default)]
struct Reading {
    chosen: Option<LetterId>,
}

fn forget(mut reading: ResMut<Reading>) {
    *reading = Reading::default();
}

enum Done {
    Choose(LetterId),
    Write,
    Reply(crate::systems::society::AccountNumber),
    TakeAll(LetterId),
    Delete(LetterId, String),
}

impl Done {
    fn apply(self, world: &mut World) {
        match self {
            Done::Choose(id) => {
                world.resource_mut::<Reading>().chosen = Some(id);
                world.write_message(OpenLetter(id));
            }
            Done::Write => draft::write_to(world, None),
            Done::Reply(to) => draft::write_to(world, Some(to)),
            Done::TakeAll(letter) => {
                world.write_message(TakeAttached {
                    letter,
                    stack: None,
                });
            }
            Done::Delete(id, title) => delete(world, id, title),
        }
    }
}

fn chosen<'a>(inbox: &'a InboxView, reading: &Reading) -> Option<&'a InboxRow> {
    let chosen = reading.chosen?;
    inbox.rows.iter().find(|row| row.id == chosen)
}

fn list(ui: &mut Ui, world: &mut World, words: &MailMessages, inbox: &InboxView) -> Option<Done> {
    let room = world
        .resource::<GameSettings>()
        .mail
        .map_or(0, |rules| rules.inbox);
    let mut done = None;
    let heading = words.inbox_heading(inbox.held, room);
    sheet::section_with(ui, &heading, |ui| {
        if controls::primary(ui, words.write()).clicked() {
            done = Some(Done::Write);
        }
    });
    if !inbox.whole {
        caption(ui, words.reading_inbox());
        ui.ctx().request_repaint();
        return done;
    }
    if inbox.rows.is_empty() {
        ui.add_space(GAP);
        ui.label(face::body(words.nothing_yet()));
        caption(ui, words.nothing_note());
        return done;
    }
    let chosen = world.resource::<Reading>().chosen;
    let clock = LocalClock::of_this_machine();
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for row in &inbox.rows {
            ui.push_id(("letter", row.id.0), |ui| {
                if list_row(ui, world, words, row, chosen == Some(row.id), clock) {
                    done = Some(Done::Choose(row.id));
                }
            });
        }
    });
    done
}

/// A letter's row in the inbox. Whether the player chose it.
fn list_row(
    ui: &mut Ui,
    world: &World,
    words: &MailMessages,
    row: &InboxRow,
    selected: bool,
    clock: LocalClock,
) -> bool {
    let state = state(words, row);
    let look = controls::RowLook {
        selected,
        faint: row.read,
        ..controls::RowLook::default()
    };
    let shown = controls::row(ui, look, |ui| {
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(face::small(clock.label(row.sent)).color(ink::INK3));
                ui.with_layout(Layout::top_down(Align::Min), |ui| {
                    let title = if row.read {
                        face::body(row.title.as_str())
                    } else {
                        face::bold(row.title.as_str())
                    };
                    ui.add(
                        egui::Label::new(title)
                            .truncate()
                            .show_tooltip_when_elided(false),
                    );
                    ui.add(
                        egui::Label::new(face::small(state.as_str()).color(ink::INK3))
                            .truncate()
                            .show_tooltip_when_elided(false),
                    );
                });
            });
        });
    });
    let response = shown
        .response
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| {
        let said = words.row_named(&state, &row.title);
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, said)
    });
    map::clicked(world, InputAction::Select, &response)
}

/// Who a letter is from, what is attached and whether it was read, in words.
fn state(words: &MailMessages, row: &InboxRow) -> String {
    let mut says = vec![sender(words, row).to_owned()];
    let attached = row.attached.len() + row.set_aside as usize;
    if attached > 0 {
        says.push(words.attached_count(attached));
    }
    if !row.read {
        says.push(words.unread().to_owned());
    }
    says.join(" · ")
}

/// Who wrote `row`, as its line in the list says: the system, or the character's name.
fn sender<'a>(words: &'a MailMessages, row: &'a InboxRow) -> &'a str {
    match &row.from {
        InboxSender::System => words.system(),
        InboxSender::Character(from) => &from.name,
    }
}

/// Who wrote `row`, as the open letter says it.
fn written_by(words: &MailMessages, row: &InboxRow) -> String {
    match &row.from {
        InboxSender::System => words.system_letter().to_owned(),
        InboxSender::Character(from) => words.from(&from.name),
    }
}

fn reading(
    ui: &mut Ui,
    world: &mut World,
    words: &MailMessages,
    inbox: &InboxView,
    society: &SocietyView,
) -> Option<Done> {
    let Some(row) = chosen(inbox, world.resource::<Reading>()) else {
        if inbox.whole && !inbox.rows.is_empty() {
            form::nothing_selected(ui, words.choose_to_read());
        }
        return None;
    };
    let mut done = None;
    ui.label(face::bold(row.title.as_str()));
    let sent = LocalClock::of_this_machine().label(row.sent);
    caption(ui, &words.sent_line(written_by(words, row), sent));
    ui.add_space(GAP);
    match world.resource::<LetterTexts>().of(row.id) {
        Some(text) => {
            ui.add(egui::Label::new(face::body(text)).wrap());
        }
        None => caption(ui, words.opening()),
    }
    let attached = row.attached.len() + row.set_aside as usize;
    if attached > 0 {
        ui.add_space(GAP);
        form::section_count(ui, words.attached(), attached);
        done = done.take().or(attachments(ui, world, words, row));
        if row.set_aside > 0 {
            caption(ui, &words.set_aside(row.set_aside));
        }
    }
    ui.add_space(GAP * 2.0);
    ui.horizontal(|ui| {
        if let InboxSender::Character(from) = &row.from {
            if !draft::may_write_to(society, from.account) {
                let why = words.cannot_reply(&from.name);
                controls::faded(ui, words.reply()).on_hover_text(why);
            } else if controls::button(ui, words.reply()).clicked() {
                done = Some(Done::Reply(from.account));
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if attached > 0 {
                ui.label(face::small(words.take_first()).color(ink::INK3));
                controls::faded(ui, words.delete());
            } else if controls::danger(ui, words.delete()).clicked() {
                done = Some(Done::Delete(row.id, row.title.clone()));
            }
        });
    });
    done
}

/// The stacks attached to the letter `row` is, each taken with the input that places an item,
/// and all of them at once.
fn attachments(
    ui: &mut Ui,
    world: &mut World,
    words: &MailMessages,
    row: &InboxRow,
) -> Option<Done> {
    let placed: Vec<Placed> = row
        .attached
        .iter()
        .map(|&stack| Placed {
            stack,
            short: false,
        })
        .collect();
    let pane = TransferPane {
        placed: &placed,
        slots: placed.len() as u32,
        named: words.attached(),
        transfer: Some(ItemTransfer::of::<ReceivedAttachments>()),
    };
    transfer::transfer_pane(ui, world, pane);
    (!row.attached.is_empty() && controls::primary(ui, words.take_all()).clicked())
        .then_some(Done::TakeAll(row.id))
}

/// What is attached to the open letter, as a window items move out of into the bag.
struct ReceivedAttachments;

impl TransferWindow for ReceivedAttachments {
    fn open(_: &World) -> bool {
        false
    }

    fn put(_: &mut World, _: u32, _: u32) -> bool {
        false
    }

    fn take(world: &mut World, index: usize, count: u32) -> bool {
        let Some(letter) = world.resource::<Reading>().chosen else {
            return false;
        };
        world.write_message(TakeAttached {
            letter,
            stack: Some(TakenStack {
                index: index as u32,
                count,
            }),
        });
        true
    }

    fn aside(_: &World, _: u32, _: crate::systems::item::ItemStack) -> u32 {
        0
    }
}

/// Asks the player whether to delete the letter titled `title`, and deletes it once they confirm.
fn delete(world: &mut World, id: LetterId, title: String) {
    let content = world.resource::<Content>().clone();
    let messages = messages::of(&content);
    hud::confirm(
        world,
        Confirm {
            title: messages.mail.delete_title(&title).into(),
            body: vec![messages.mail.delete_body().into()],
            confirm: messages.mail.delete().to_owned(),
            cancel: messages.friends.keep().to_owned(),
            answer: Box::new(move |world, deleted| {
                if deleted {
                    world.write_message(DeleteLetter(id));
                }
            }),
        },
    );
}

fn caption(ui: &mut Ui, text: &str) {
    form::caption(ui, text, ink::INK2);
}

const LIST_WIDTH: f32 = 380.0;
const GAP: f32 = 8.0;
