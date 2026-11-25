//! A call as its players see it: the roster window, where the leader picks who goes and enters,
//! and the call card, where each member answers.

use bevy::prelude::*;
use ui::egui::{self, Align, Layout, Pos2, Rect, Ui, Vec2};
use ui::face::TextRole;
use ui::{controls, dialog, face, ink, sheet};

use super::InstanceMessages;
use super::call;
use super::roster::{
    CallAnswer, CallReply, CallRequest, CallRowShown, InstanceCallShown, RosterRowShown,
};
use crate::core::content::Content;
use crate::core::text::LineText;
use crate::core::text::TextLook;
use crate::core::time::{self, UnixMillis};
use crate::core::units::Seconds;
use crate::systems::GameSettings;
use crate::systems::hud::{self, Centred, HudWidget, WindowKey};
use crate::systems::input::map::{self, InputAction, InputMap};
use crate::systems::input::scope;
use crate::systems::item::inline;
use crate::systems::mention::Mention;
use crate::systems::messages;
use crate::systems::screen::ClientScreen;
use crate::systems::settings::Placed;
use crate::systems::social_request::card::time_left;

pub const INSTANCE_CALL: &str = "instance_call";

pub(crate) fn plugin(app: &mut App) {
    app.add_systems(Update, answer_keys.run_if(in_state(ClientScreen::Area)));
}

/// The roster window: a sheet up for a leader while their call is open. Closing it calls the
/// party off.
pub(crate) fn draw(world: &mut World, gui: &egui::Context) {
    let Some(now) = time::server_now(world) else {
        return;
    };
    let content = world.resource::<Content>().clone();
    let window = Centred {
        key: WindowKey::Roster,
        of: Placed::Roster,
        body: Vec2::new(WIDTH, BODY_HEIGHT),
        top: 120.0,
    };
    let title = |leading: &InstanceCallShown| {
        messages::of(&content)
            .instance
            .roster_title(&leading.area.get(&content).name)
    };
    let drawn = hud::centred(
        world,
        gui,
        window,
        call(world).filter(|call| call.leads),
        title,
        |ui, _, leading| roster(ui, &content, leading, now),
    );
    let Some(drawn) = drawn else {
        return;
    };
    let asked = if drawn.closed {
        Some(CallRequest::CallOff)
    } else {
        drawn.inner
    };
    if let Some(asked) = asked {
        world.write_message(asked);
    }
}

/// What the roster window holds: how long the call stays open, a row for each member, and the
/// slots taken beside Enter.
fn roster(
    ui: &mut Ui,
    content: &Content,
    shown: &InstanceCallShown,
    now: UnixMillis,
) -> Option<CallRequest> {
    let mut asked = None;
    let padding = egui::Margin::symmetric(sheet::PANE_PAD.x as i8, sheet::PANE_PAD.y as i8);
    egui::Frame::NONE.inner_margin(padding).show(ui, |ui| {
        let messages = &messages::of(content).instance;
        let closes = messages.closes_in(time_left(&messages::of(content).time, shown.ends, now));
        controls::pill(ui, &closes, controls::Pill::Ink);
        ui.add_space(6.0);
        let free = shown.taken < shown.players.most;
        for row in &shown.rows {
            if let Some(request) = roster_row(ui, content, row, free) {
                asked = Some(request);
            }
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let ready = shown.ready >= shown.players.least;
                if ui
                    .add_enabled_ui(ready, |ui| controls::primary(ui, messages.enter()))
                    .inner
                    .clicked()
                {
                    asked = Some(CallRequest::Enter);
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    let slots = if free {
                        messages.slots(shown.players.most, shown.taken)
                    } else {
                        messages.slots_full(shown.players.most, shown.taken)
                    };
                    ui.label(face::bold(slots));
                });
            });
        });
        if shown.ready < shown.players.least {
            let needs = messages.needs_ready(shown.ready, shown.players.least);
            ui.with_layout(Layout::top_down(Align::Max), |ui| {
                ui.label(face::small(needs).color(ink::VERMILION));
            });
        }
    });
    asked
}

/// The call card, shown to a member of a party whose leader called them.
pub struct CallCardWidget;

impl HudWidget for CallCardWidget {
    fn fallback(&self, _: &World, screen: Rect) -> Pos2 {
        screen.right_top() + Vec2::new(-(WIDTH + hud::MARGIN), FROM_TOP)
    }

    fn hidden(&self, world: &World) -> bool {
        !card_up(world)
    }

    fn show(&self, ui: &mut Ui, world: &mut World) -> Option<Rect> {
        let call = call(world).filter(|call| !call.leads)?;
        let row = call.rows.first()?.clone();
        let now = time::server_now(world)?;
        let content = world.resource::<Content>().clone();
        let messages = &messages::of(&content).instance;
        let lasts = world
            .resource::<GameSettings>()
            .party
            .map_or(Seconds(1.0), |party| party.call_lasts);
        let keys = world.resource::<InputMap>();
        let cap = |action| {
            scope::reads(world, action)
                .then(|| keys.cap(action))
                .flatten()
        };
        let (join_key, decline_key) = (
            cap(InputAction::AcceptRequest),
            cap(InputAction::DenyRequest),
        );
        let leader = call.leader.name.clone();
        let area = Mention::Area(call.area).span(&content);
        let mut replied = None;
        dialog::note(ui, WIDTH, |ui| {
            ui.horizontal(|ui| {
                ui.label(face::small(messages.call_heading()).color(ink::INK3));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(face::small_faint(time_left(
                        &messages::of(&content).time,
                        call.ends,
                        now,
                    )));
                });
            });
            let says = match row.answer {
                CallAnswer::Deciding => {
                    messages.call_open(area.clone(), &leader, call.players.most)
                }
                _ => messages.call(area.clone(), &leader),
            };
            let look = TextLook::role(TextRole::Body, ink::INK);
            inline::show_line(ui, world, &says, look, None);
            if row.answer == CallAnswer::Deciding {
                let left = (call.ends.since(now).0 / lasts.0).clamp(0.0, 1.0);
                ui.add(egui::ProgressBar::new(left).desired_height(6.0));
            }
            ui.add_space(4.0);
            let (pill, note) = state(messages, &row, &leader);
            ui.horizontal_wrapped(|ui| {
                if let Some((said, look)) = pill {
                    controls::pill(ui, said, look);
                }
                if let Some(note) = note {
                    let look = TextLook::role(TextRole::Small, ink::INK2);
                    inline::show_line(ui, world, &LineText::plain(note), look, None);
                }
            });
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if row.answer != CallAnswer::Declined {
                        let label = match row.answer {
                            CallAnswer::Deciding => messages.not_this_time(),
                            _ => messages.withdraw(),
                        };
                        if controls::keyed(ui, label, decline_key.as_ref(), true).clicked() {
                            replied = Some(CallReply::Decline);
                        }
                    }
                    if !matches!(row.answer, CallAnswer::Joined { .. })
                        && controls::keyed(ui, messages.join(), join_key.as_ref(), true).clicked()
                    {
                        replied = Some(CallReply::Join);
                    }
                });
            });
            if row.answer == CallAnswer::Deciding {
                ui.label(face::small(messages.change_answer()).color(ink::INK3));
            }
        });
        if let Some(replied) = replied {
            world.write_message(replied);
        }
        None
    }
}

/// Whether the call card is up, which the keys that answer requests answer first.
pub fn card_up(world: &World) -> bool {
    call(world).is_some_and(|call| !call.leads)
}

/// Calls off the call the player leads, if they lead one.
/// Whether the player leads a call they can call off.
pub fn leads_call(world: &World) -> bool {
    call(world).is_some_and(|call| call.leads)
}

pub fn call_off(world: &mut World) -> bool {
    if !leads_call(world) {
        return false;
    }
    world.write_message(CallRequest::CallOff);
    true
}

/// One row of the roster window: the member's name, their state in words, and the button that
/// picks or unpicks them where the leader may. What the button asks, once pressed.
fn roster_row(
    ui: &mut Ui,
    content: &Content,
    row: &CallRowShown,
    free: bool,
) -> Option<CallRequest> {
    let messages = &messages::of(content).instance;
    let mut asked = None;
    let far = matches!(
        row.shown,
        RosterRowShown::TooFar { .. } | RosterRowShown::Away | RosterRowShown::Fallen
    );
    ui.horizontal(|ui| {
        ui.set_min_height(controls::CONTROL_HEIGHT);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let number = row.character.account;
            match (row.shown, row.answer) {
                (RosterRowShown::Leading, _) => {}
                (_, CallAnswer::Joined { picked: true }) => {
                    if controls::button(ui, messages.unpick()).clicked() {
                        asked = Some(CallRequest::Unpick(number));
                    }
                }
                (RosterRowShown::Benched, _) => {
                    let picks = ui.add_enabled_ui(free, |ui| controls::button(ui, messages.pick()));
                    if picks.inner.clicked() {
                        asked = Some(CallRequest::Pick(number));
                    }
                }
                _ => {}
            }
            let (said, look) = row_pill(messages, row.shown);
            controls::pill(ui, said, look);
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let name = match (row.shown, far) {
                    (RosterRowShown::Leading, _) => {
                        face::bold(messages.you_named(&row.character.name))
                    }
                    (RosterRowShown::TooFar { area }, _) => {
                        face::body(messages.in_area(&area.get(content).name, &row.character.name))
                            .color(ink::INK3)
                    }
                    (_, true) => face::body(row.character.name.as_str()).color(ink::INK3),
                    (_, false) => face::body(row.character.name.as_str()),
                };
                ui.add(
                    egui::Label::new(name)
                        .truncate()
                        .show_tooltip_when_elided(false),
                );
            });
        });
    });
    asked
}

fn row_pill(messages: &InstanceMessages, shown: RosterRowShown) -> (&str, controls::Pill) {
    use controls::Pill;
    match shown {
        RosterRowShown::Leading => (messages.going_leader(), Pill::Verdigris),
        RosterRowShown::Going => (messages.going(), Pill::Verdigris),
        RosterRowShown::Benched => (messages.on_bench(), Pill::Gold),
        RosterRowShown::Deciding { .. } => (messages.deciding(), Pill::Ink),
        RosterRowShown::NotGoing => (messages.not_going(), Pill::Ink),
        RosterRowShown::TooFar { .. } => (messages.too_far(), Pill::Gold),
        RosterRowShown::Away => (messages.away_from_game(), Pill::Ink),
        RosterRowShown::Fallen => (messages.fallen(), Pill::Rubric),
    }
}

/// What a member's card says of where they stand: a pill, and a line beside it where one is due.
fn state<'m>(
    messages: &'m InstanceMessages,
    row: &CallRowShown,
    leader: &str,
) -> (Option<(&'m str, controls::Pill)>, Option<String>) {
    use controls::Pill;
    match row.shown {
        RosterRowShown::Deciding { .. } => (None, None),
        RosterRowShown::Going => (Some((messages.you_go(), Pill::Verdigris)), None),
        RosterRowShown::Benched => (
            Some((messages.on_bench(), Pill::Gold)),
            Some(messages.benched(leader)),
        ),
        RosterRowShown::TooFar { .. } | RosterRowShown::Away => (
            Some((messages.too_far(), Pill::Gold)),
            Some(messages.out_of_sight(leader)),
        ),
        shown => (Some(row_pill(messages, shown)), None),
    }
}

/// The keys that answer requests answer the call card first: one joins, the other declines or
/// withdraws.
fn answer_keys(world: &mut World) {
    let Some(row) = call(world)
        .filter(|call| !call.leads)
        .and_then(|call| call.rows.first().cloned())
    else {
        return;
    };
    let reply = if map::just_pressed(world, InputAction::AcceptRequest) {
        CallReply::Join
    } else if map::just_pressed(world, InputAction::DenyRequest) {
        CallReply::Decline
    } else {
        return;
    };
    let same = matches!(
        (reply, row.answer),
        (CallReply::Join, CallAnswer::Joined { .. }) | (CallReply::Decline, CallAnswer::Declined)
    );
    if !same {
        world.write_message(reply);
    }
}

const WIDTH: f32 = 380.0;
const BODY_HEIGHT: f32 = 280.0;
const FROM_TOP: f32 = 190.0;
