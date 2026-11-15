//! The Messages page of the Social tab: the inbox, the message chosen in it with what is
//! attached, and the message being written.

use std::collections::HashMap;

use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ui::egui::{self, Align, Layout, Sense, Ui, Vec2};
use ui::{controls, face, form, ink, sheet, words};

use super::draft::{self, ShownDraft};
use super::{
    DeleteMessage, InboxRow, InboxView, MessageId, MessageText, OpenMessage, TakeAttached,
};
use crate::core::content::Content;
use crate::core::time::LocalClock;
use crate::systems::GameSettings;
use crate::systems::hud::{self, Confirm};
use crate::systems::input::map::{self, InputAction};
use crate::systems::item;
use crate::systems::item::widget::{SlotLook, slot};
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;
use crate::systems::society::SocietyView;
use crate::systems::society::tab::SocialPage;

pub(crate) const TITLE: &str = "Messages";

pub struct MessagesPage;

impl SocialPage for MessagesPage {
    fn title(&self) -> &'static str {
        TITLE
    }

    fn on(&self, world: &World) -> bool {
        world.resource::<GameSettings>().messages.is_some()
    }

    fn waiting(&self, world: &World) -> u32 {
        inbox(world).map_or(0, |inbox| inbox.unread())
    }

    fn hints(&self, world: &World) -> Vec<(InputAction, &'static str)> {
        if draft::writing(world) {
            vec![
                (InputAction::PlaceItem, "attach"),
                (InputAction::PlaceStack, "attach the stack"),
            ]
        } else {
            vec![
                (InputAction::Select, "choose"),
                (InputAction::PlaceItem, "take one"),
            ]
        }
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
        let inbox = inbox(world).cloned().unwrap_or_default();
        hear(world, &inbox);
        let room = ui.available_rect_before_wrap();
        let ([list_at], reading_at) = sheet::columns(ui, room, [LIST_WIDTH]);
        let mut done = None;
        let area = egui::ScrollArea::vertical().id_salt("inbox");
        sheet::scroll_pane(ui, list_at, area, |ui| {
            done = done.take().or(list(ui, world, &inbox));
        });
        let area = egui::ScrollArea::vertical().id_salt("message read");
        sheet::scroll_pane(ui, reading_at, area, |ui| {
            done = done.take().or(reading(ui, world, &inbox, &society));
        });
        ui.allocate_rect(room, Sense::hover());
        if let Some(done) = done {
            done.apply(world);
        }
    }
}

/// What the Messages page shows, for a driver of the client.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
pub struct ShownMessages {
    /// The inbox, the newest message first.
    pub inbox: Vec<ShownMessage>,
    /// The message chosen in it.
    pub open: Option<ShownOpenMessage>,
    /// The message being written.
    pub draft: Option<ShownDraft>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ShownMessage {
    pub title: String,
    /// The player who wrote it. A message from the game has none.
    pub from: Option<String>,
    pub read: bool,
    /// How many stacks are attached.
    pub attached: u32,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ShownOpenMessage {
    pub title: String,
    /// Its text, once the server has sent it.
    pub text: Option<String>,
    /// Each stack attached, as its count and its item's name.
    pub attached: Vec<String>,
}

pub fn shown(world: &World) -> ShownMessages {
    let Some(inbox) = inbox(world) else {
        return ShownMessages::default();
    };
    let content = world.resource::<Content>();
    let reading = world.resource::<Reading>();
    ShownMessages {
        inbox: inbox
            .rows
            .iter()
            .map(|row| ShownMessage {
                title: row.title.clone(),
                from: row.from.as_ref().map(|from| from.name.clone()),
                read: row.read,
                attached: row.attached.len() as u32,
            })
            .collect(),
        open: chosen(inbox, reading).map(|row| ShownOpenMessage {
            title: row.title.clone(),
            text: reading.texts.get(&row.id).cloned(),
            attached: row
                .attached
                .iter()
                .map(|stack| stack.describe(content))
                .collect(),
        }),
        draft: draft::shown(world),
    }
}

pub(crate) fn plugin(app: &mut App) {
    draft::plugin(app);
    app.init_resource::<Reading>()
        .add_systems(OnExit(ClientScreen::Area), forget);
}

/// What the player reads: the message chosen, and the texts the server has sent.
#[derive(Resource, Default)]
struct Reading {
    chosen: Option<MessageId>,
    texts: HashMap<MessageId, String>,
}

fn forget(mut reading: ResMut<Reading>) {
    *reading = Reading::default();
}

enum Done {
    Choose(MessageId),
    Write,
    Reply(crate::systems::society::AccountNumber),
    Take(MessageId, Option<u32>),
    Delete(MessageId, String),
}

impl Done {
    fn apply(self, world: &mut World) {
        match self {
            Done::Choose(id) => {
                world.resource_mut::<Reading>().chosen = Some(id);
                world.write_message(OpenMessage(id));
            }
            Done::Write => draft::write_to(world, None),
            Done::Reply(to) => draft::write_to(world, Some(to)),
            Done::Take(message, stack) => {
                world.write_message(TakeAttached { message, stack });
            }
            Done::Delete(id, title) => delete(world, id, title),
        }
    }
}

fn inbox(world: &World) -> Option<&InboxView> {
    session::my_character(world)?.get::<InboxView>()
}

fn chosen<'a>(inbox: &'a InboxView, reading: &Reading) -> Option<&'a InboxRow> {
    let chosen = reading.chosen?;
    inbox.rows.iter().find(|row| row.id == chosen)
}

/// Takes the texts the server sent, and forgets those of messages that are gone.
fn hear(world: &mut World, inbox: &InboxView) {
    let sent: Vec<MessageText> = world
        .resource_mut::<Messages<MessageText>>()
        .drain()
        .collect();
    let mut reading = world.resource_mut::<Reading>();
    for MessageText { id, text } in sent {
        reading.texts.insert(id, text);
    }
    if inbox.whole {
        reading
            .texts
            .retain(|id, _| inbox.rows.iter().any(|row| row.id == *id));
    }
}

fn list(ui: &mut Ui, world: &mut World, inbox: &InboxView) -> Option<Done> {
    let room = world
        .resource::<GameSettings>()
        .messages
        .map_or(0, |rules| rules.inbox);
    let mut done = None;
    let heading = format!("Inbox · {} of {room}", inbox.held);
    sheet::section_with(ui, &heading, |ui| {
        if controls::primary(ui, "Write a message").clicked() {
            done = Some(Done::Write);
        }
    });
    if !inbox.whole {
        caption(ui, "Reading your messages");
        ui.ctx().request_repaint();
        return done;
    }
    if inbox.rows.is_empty() {
        ui.add_space(GAP);
        ui.label(face::body("Nothing has been sent to you yet."));
        caption(
            ui,
            "Friends and party members can write to you, and rewards that find your inventory \
             full wait here.",
        );
        return done;
    }
    let chosen = world.resource::<Reading>().chosen;
    let clock = LocalClock::of_this_machine();
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for row in &inbox.rows {
            ui.push_id(("message", row.id.0), |ui| {
                if list_row(ui, world, row, chosen == Some(row.id), clock) {
                    done = Some(Done::Choose(row.id));
                }
            });
        }
    });
    done
}

/// A message's row in the inbox. Whether the player chose it.
fn list_row(ui: &mut Ui, world: &World, row: &InboxRow, selected: bool, clock: LocalClock) -> bool {
    let state = state(row);
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
                    ui.add(egui::Label::new(title).truncate());
                    ui.add(
                        egui::Label::new(face::small(state.as_str()).color(ink::INK3)).truncate(),
                    );
                });
            });
        });
    });
    let response = shown
        .response
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| {
        let said = format!("{}, {state}", row.title);
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, said)
    });
    map::clicked(world, InputAction::Select, &response)
}

/// Who a message is from, what is attached and whether it was read, in words.
fn state(row: &InboxRow) -> String {
    let mut says = vec![sender(row).to_owned()];
    let attached = row.attached.len() + row.set_aside as usize;
    if attached > 0 {
        says.push(format!("{attached} attached"));
    }
    if !row.read {
        says.push("Unread".to_owned());
    }
    says.join(" · ")
}

fn sender(row: &InboxRow) -> &str {
    row.from.as_ref().map_or("The game", |from| &from.name)
}

fn reading(
    ui: &mut Ui,
    world: &mut World,
    inbox: &InboxView,
    society: &SocietyView,
) -> Option<Done> {
    let Some(row) = chosen(inbox, world.resource::<Reading>()) else {
        if inbox.whole && !inbox.rows.is_empty() {
            form::nothing_selected(ui, "Choose a message to read it");
        }
        return None;
    };
    let mut done = None;
    ui.label(face::bold(row.title.as_str()));
    let sent = LocalClock::of_this_machine().label(row.sent);
    caption(ui, &format!("From {} · {sent}", sender(row)));
    ui.add_space(GAP);
    match world.resource::<Reading>().texts.get(&row.id) {
        Some(text) => {
            ui.add(egui::Label::new(face::body(text.as_str())).wrap());
        }
        None => caption(ui, "Opening"),
    }
    let attached = row.attached.len() + row.set_aside as usize;
    if attached > 0 {
        ui.add_space(GAP);
        form::section_count(ui, "Attached", attached);
        done = done.take().or(attachments(ui, world, row));
        if row.set_aside > 0 {
            let kept = words::counted(row.set_aside, "stacks");
            caption(
                ui,
                &format!(
                    "{kept} of items the game no longer has wait here until it has them again."
                ),
            );
        }
    }
    ui.add_space(GAP * 2.0);
    ui.horizontal(|ui| {
        let friend = row
            .from
            .as_ref()
            .filter(|from| draft::may_write_to(society, from.account));
        match (&row.from, friend) {
            (_, Some(from)) => {
                if controls::button(ui, "Reply").clicked() {
                    done = Some(Done::Reply(from.account));
                }
            }
            (Some(from), None) => {
                let why = format!("{} is neither a friend nor in your party", from.name);
                controls::faded(ui, "Reply").on_hover_text(why);
            }
            (None, None) => {
                controls::faded(ui, "Reply").on_hover_text("The game takes no messages");
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if attached > 0 {
                ui.label(face::small("Take what is attached first").color(ink::INK3));
                controls::faded(ui, "Delete");
            } else if controls::danger(ui, "Delete").clicked() {
                done = Some(Done::Delete(row.id, row.title.clone()));
            }
        });
    });
    done
}

/// The stacks attached to the message `row` is, each taken with the input that places an item,
/// and all of them at once.
fn attachments(ui: &mut Ui, world: &mut World, row: &InboxRow) -> Option<Done> {
    let content = world.resource::<Content>().clone();
    let mut done = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::splat(SLOT_GAP);
        for (index, stack) in row.attached.iter().enumerate() {
            let def = stack.item.get(&content);
            let look = SlotLook {
                count: stack.count,
                ..SlotLook::of(&content, stack.item)
            };
            let response = slot(ui, world, look, Sense::click());
            response.widget_info(|| {
                let said = format!("Attached: {}", stack.describe(&content));
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, said)
            });
            let takes = map::clicked(world, InputAction::PlaceItem, &response)
                || map::clicked(world, InputAction::Select, &response);
            if takes {
                done = Some(Done::Take(row.id, Some(index as u32)));
            }
            item::card::chat_shortcut(world, &response, &[stack.item]);
            hud::tooltip(
                &response,
                world,
                &def.name,
                &["Click to take it".to_owned()],
                None,
            );
        }
        if !row.attached.is_empty() && controls::primary(ui, "Take all").clicked() {
            done = Some(Done::Take(row.id, None));
        }
    });
    done
}

/// Asks the player whether to delete the message titled `title`, and deletes it once they confirm.
fn delete(world: &mut World, id: MessageId, title: String) {
    hud::confirm(
        world,
        Confirm {
            title: format!("Delete \"{title}\"?"),
            body: vec!["It cannot be brought back.".to_owned()],
            confirm: "Delete".to_owned(),
            cancel: "Keep".to_owned(),
            answer: Box::new(move |world, deleted| {
                if deleted {
                    world.write_message(DeleteMessage(id));
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
const SLOT_GAP: f32 = 6.0;
