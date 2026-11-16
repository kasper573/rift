//! A call as its players see it: the roster window, where the leader picks who goes and enters,
//! and the call card, where each member answers.

use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ui::egui::{self, Align, Layout, Pos2, Rect, Ui, Vec2};
use ui::face::TextRole;
use ui::{controls, dialog, face, ink};

use super::roster::{
    CallAnswer, CallReply, CallRequest, CallRowShown, InstanceCallShown, RosterRowShown,
};
use crate::core::content::Content;
use crate::core::text::{self, TextLook};
use crate::core::time;
use crate::core::units::Seconds;
use crate::systems::GameSettings;
use crate::systems::hud::{self, HudWidget};
use crate::systems::input::map::{self, InputAction, InputMap};
use crate::systems::mention::Mention;
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;
use crate::systems::social_request::card::time_left;
use crate::systems::society::SocietyView;
use crate::systems::text::LineText;

pub const INSTANCE_ROSTER: &str = "instance_roster";
pub const INSTANCE_CALL: &str = "instance_call";

pub(crate) fn plugin(app: &mut App) {
    app.add_systems(Update, answer_keys.run_if(in_state(ClientScreen::Area)));
}

/// The roster window, shown to a leader while their call is open.
pub struct RosterWindowWidget;

impl HudWidget for RosterWindowWidget {
    fn fallback(&self, _: &World, screen: Rect) -> Pos2 {
        Pos2::new(screen.center().x - WIDTH / 2.0, screen.top() + 120.0)
    }

    fn hidden(&self, world: &World) -> bool {
        call(world).is_none_or(|call| !call.leads)
    }

    fn show(&self, ui: &mut Ui, world: &mut World) -> Option<Rect> {
        let call = call(world).filter(|call| call.leads)?;
        let now = time::server_now(world)?;
        let content = world.resource::<Content>().clone();
        let mut asked = None;
        dialog::note(ui, WIDTH, |ui| {
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let closes = format!("Closes in {}", time_left(call.ends, now));
                    controls::pill(ui, &closes, controls::Pill::Ink);
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        let title = Mention::Area(call.area)
                            .line(&content)
                            .then(" · party roster");
                        let look = TextLook::role(TextRole::Body, ink::INK);
                        text::show(ui, world, &title.styled(), look.bold(), None);
                    });
                });
            });
            ui.add_space(6.0);
            let free = call.taken < call.players.most;
            for row in &call.rows {
                if let Some(request) = roster_row(ui, &content, row, free) {
                    asked = Some(request);
                }
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let ready = call.ready >= call.players.least;
                    if ui
                        .add_enabled_ui(ready, |ui| controls::primary(ui, "Enter"))
                        .inner
                        .clicked()
                    {
                        asked = Some(CallRequest::Enter);
                    }
                    if controls::button(ui, "Call off").clicked() {
                        asked = Some(CallRequest::CallOff);
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        let full = if free { "" } else { " · full" };
                        let slots = format!("{} of {} slots{full}", call.taken, call.players.most);
                        ui.label(face::bold(slots));
                    });
                });
            });
            if call.ready < call.players.least {
                let needs = format!(
                    "Needs at least {} players. {} {} ready.",
                    call.players.least,
                    call.ready,
                    if call.ready == 1 { "is" } else { "are" }
                );
                ui.with_layout(Layout::top_down(Align::Max), |ui| {
                    ui.label(face::small(needs).color(ink::VERMILION));
                });
            }
        });
        if let Some(asked) = asked {
            world.write_message(asked);
        }
        None
    }
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
        let settings = world.resource::<GameSettings>();
        let words = settings.words.clone();
        let lasts = settings
            .party
            .map_or(Seconds(1.0), |party| party.call_lasts);
        let keys = world.resource::<InputMap>();
        let (join_key, decline_key) = (
            keys.name(InputAction::AcceptRequest),
            keys.name(InputAction::DenyRequest),
        );
        let leader = call.leader.name.clone();
        let area = Mention::Area(call.area).span(&content);
        let mut replied = None;
        dialog::note(ui, WIDTH, |ui| {
            ui.horizontal(|ui| {
                ui.label(face::small("INSTANCE CALL").color(ink::INK3));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(face::small_faint(time_left(call.ends, now)));
                });
            });
            let mut says = LineText::plain(words.instance_call.as_str())
                .filling("{leader}", leader.clone())
                .filling("{area}", area.clone());
            if row.answer == CallAnswer::Deciding {
                says = says.then(format!(" Up to {} go.", call.players.most));
            }
            let look = TextLook::role(TextRole::Body, ink::INK);
            text::show(ui, world, &says.styled(), look, None);
            if row.answer == CallAnswer::Deciding {
                let left = (call.ends.since(now).0 / lasts.0).clamp(0.0, 1.0);
                ui.add(egui::ProgressBar::new(left).desired_height(6.0));
            }
            ui.add_space(4.0);
            let (pill, note) = state(&row, &words.instance_benched, &words.instance_out_of_sight);
            ui.horizontal_wrapped(|ui| {
                if let Some((said, look)) = pill {
                    controls::pill(ui, said, look);
                }
                if let Some(note) = note {
                    let note = LineText::plain(note).filling("{leader}", leader.clone());
                    let look = TextLook::role(TextRole::Small, ink::INK2);
                    text::show(ui, world, &note.styled(), look, None);
                }
            });
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if row.answer != CallAnswer::Declined {
                        let label = match row.answer {
                            CallAnswer::Deciding => "Not this time",
                            _ => "Withdraw",
                        };
                        if controls::keyed(ui, label, decline_key.as_deref(), true).clicked() {
                            replied = Some(CallReply::Decline);
                        }
                    }
                    if !matches!(row.answer, CallAnswer::Joined { .. })
                        && controls::keyed(ui, "Join", join_key.as_deref(), true).clicked()
                    {
                        replied = Some(CallReply::Join);
                    }
                });
            });
            if row.answer == CallAnswer::Deciding {
                let note = "You can change your answer until the party enters.";
                ui.label(face::small(note).color(ink::INK3));
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
pub fn call_off(world: &mut World) -> bool {
    if call(world).is_none_or(|call| !call.leads) {
        return false;
    }
    world.write_message(CallRequest::CallOff);
    true
}

/// A call as the roster window or the card shows it, for a driver of the client.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ShownInstanceCall {
    pub area: String,
    pub leader: String,
    /// Whether the player leads it, and sees the roster window rather than the card.
    pub leads: bool,
    pub left: Seconds,
    pub least: u32,
    pub most: u32,
    /// How many slots are taken: the leader and everyone picked.
    pub taken: u32,
    /// How many would go if the leader entered now.
    pub ready: u32,
    /// The leader first, then the members; to a member, only their own row.
    pub rows: Vec<ShownCallRow>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ShownCallRow {
    pub name: String,
    pub state: CallRowState,
    /// The area they play in, while they are too far.
    pub area: Option<String>,
    pub answer: ShownCallAnswer,
    pub picked: bool,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CallRowState {
    Leading,
    Going,
    Benched,
    Deciding,
    NotGoing,
    TooFar,
    Away,
    Fallen,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShownCallAnswer {
    Deciding,
    Joined,
    Declined,
}

pub fn shown(world: &World) -> Option<ShownInstanceCall> {
    let call = call(world)?;
    let now = time::server_now(world)?;
    let content = world.resource::<Content>();
    Some(ShownInstanceCall {
        area: call.area.get(content).name.clone(),
        leader: call.leader.name.clone(),
        leads: call.leads,
        left: call.ends.since(now),
        least: call.players.least,
        most: call.players.most,
        taken: call.taken,
        ready: call.ready,
        rows: call
            .rows
            .iter()
            .map(|row| {
                let (state, area) = match row.shown {
                    RosterRowShown::Leading => (CallRowState::Leading, None),
                    RosterRowShown::Going => (CallRowState::Going, None),
                    RosterRowShown::Benched => (CallRowState::Benched, None),
                    RosterRowShown::Deciding { .. } => (CallRowState::Deciding, None),
                    RosterRowShown::NotGoing => (CallRowState::NotGoing, None),
                    RosterRowShown::TooFar { area } => {
                        (CallRowState::TooFar, Some(area.get(content).name.clone()))
                    }
                    RosterRowShown::Away => (CallRowState::Away, None),
                    RosterRowShown::Fallen => (CallRowState::Fallen, None),
                };
                ShownCallRow {
                    name: row.player.name.clone(),
                    state,
                    area,
                    answer: match row.answer {
                        CallAnswer::Deciding => ShownCallAnswer::Deciding,
                        CallAnswer::Joined { .. } => ShownCallAnswer::Joined,
                        CallAnswer::Declined => ShownCallAnswer::Declined,
                    },
                    picked: matches!(row.answer, CallAnswer::Joined { picked: true }),
                }
            })
            .collect(),
    })
}

fn call(world: &World) -> Option<InstanceCallShown> {
    session::my_character(world)?
        .get::<SocietyView>()?
        .call
        .clone()
}

/// One row of the roster window: the member's name, their state in words, and the button that
/// picks or unpicks them where the leader may. What the button asks, once pressed.
fn roster_row(
    ui: &mut Ui,
    content: &Content,
    row: &CallRowShown,
    free: bool,
) -> Option<CallRequest> {
    let mut asked = None;
    let far = matches!(
        row.shown,
        RosterRowShown::TooFar { .. } | RosterRowShown::Away | RosterRowShown::Fallen
    );
    ui.horizontal(|ui| {
        ui.set_min_height(controls::CONTROL_HEIGHT);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let number = row.player.account;
            match (row.shown, row.answer) {
                (RosterRowShown::Leading, _) => {}
                (_, CallAnswer::Joined { picked: true }) => {
                    if controls::button(ui, "Unpick").clicked() {
                        asked = Some(CallRequest::Unpick(number));
                    }
                }
                (RosterRowShown::Benched, _) => {
                    let picks = ui.add_enabled_ui(free, |ui| controls::button(ui, "Pick"));
                    if picks.inner.clicked() {
                        asked = Some(CallRequest::Pick(number));
                    }
                }
                _ => {}
            }
            let (said, look) = row_pill(row.shown);
            controls::pill(ui, said, look);
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let name = match (row.shown, far) {
                    (RosterRowShown::Leading, _) => {
                        face::bold(format!("{} (you)", row.player.name))
                    }
                    (RosterRowShown::TooFar { area }, _) => {
                        face::body(format!("{} · {}", row.player.name, area.get(content).name))
                            .color(ink::INK3)
                    }
                    (_, true) => face::body(row.player.name.as_str()).color(ink::INK3),
                    (_, false) => face::body(row.player.name.as_str()),
                };
                ui.add(egui::Label::new(name).truncate());
            });
        });
    });
    asked
}

fn row_pill(shown: RosterRowShown) -> (&'static str, controls::Pill) {
    use controls::Pill;
    match shown {
        RosterRowShown::Leading => ("Going · leader", Pill::Verdigris),
        RosterRowShown::Going => ("Going", Pill::Verdigris),
        RosterRowShown::Benched => ("On the bench", Pill::Gold),
        RosterRowShown::Deciding { .. } => ("Deciding…", Pill::Ink),
        RosterRowShown::NotGoing => ("Not going", Pill::Ink),
        RosterRowShown::TooFar { .. } => ("Too far", Pill::Gold),
        RosterRowShown::Away => ("Away", Pill::Ink),
        RosterRowShown::Fallen => ("Fallen", Pill::Rubric),
    }
}

/// What a member's card says of where they stand: a pill, and a line beside it where one is due.
fn state<'w>(
    row: &CallRowShown,
    benched: &'w str,
    out_of_sight: &'w str,
) -> (Option<(&'static str, controls::Pill)>, Option<&'w str>) {
    use controls::Pill;
    match row.shown {
        RosterRowShown::Deciding { .. } => (None, None),
        RosterRowShown::Going => (Some(("You're going", Pill::Verdigris)), None),
        RosterRowShown::Benched => (Some(("On the bench", Pill::Gold)), Some(benched)),
        RosterRowShown::TooFar { .. } | RosterRowShown::Away => {
            (Some(("Too far", Pill::Gold)), Some(out_of_sight))
        }
        shown => (Some(row_pill(shown)), None),
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
const FROM_TOP: f32 = 190.0;
