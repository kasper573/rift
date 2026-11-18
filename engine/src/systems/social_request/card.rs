//! The request cards: a piece of the HUD that shows a player the requests they are asked, the
//! oldest in front, and takes their answer. What a player asked of others shows only on the
//! pages that list it.

use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ui::egui::{self, Align, Layout, Pos2, Rect, Ui};
use ui::{controls, dialog, face, ink};

use super::{AnswerSocialRequest, BlockCharacter, RequestAnswer, RequestMessages};
use crate::core::content::Content;
use crate::core::time::{self, UnixMillis};
use crate::core::units::Seconds;
use crate::systems::area_instance;
use crate::systems::duration::TimeMessages;
use crate::systems::hud::{self, Confirm, HudWidget};
use crate::systems::input::map::{self, InputAction, InputMap};
use crate::systems::input::scope;
use crate::systems::messages;
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;
use crate::systems::society::{RequestCard, SocietyMember, SocietyView};

pub const REQUEST_CARDS: &str = "requests";

pub struct RequestCardsWidget;

impl HudWidget for RequestCardsWidget {
    fn fallback(&self, _: &World, screen: Rect) -> Pos2 {
        Pos2::new(screen.center().x - WIDTH / 2.0, screen.top() + hud::MARGIN)
    }

    fn hidden(&self, world: &World) -> bool {
        on_hud(world).is_empty()
    }

    fn show(&self, ui: &mut Ui, world: &mut World) -> Option<Rect> {
        let cards = on_hud(world);
        let now = time::server_now(world)?;
        let content = world.resource::<Content>().clone();
        let messages = messages::of(&content);
        let words = &messages.requests;
        let (front, behind) = cards.split_first()?;
        ui.spacing_mut().item_spacing.y = CARD_GAP;
        let done = dialog::note(ui, WIDTH, |ui| {
            front_card(ui, world, words, &messages.time, front, now)
        });
        if let Some(next) = behind.first() {
            ui.with_layout(Layout::top_down(Align::Max), |ui| {
                let left = time_left(&messages.time, next.until, now);
                ui.label(face::small(format!("{} · {left}", next.says)).color(ink::LIGHT));
                if behind.len() > 1 {
                    let waiting = words.more_waiting(behind.len() - 1);
                    ui.label(face::small(waiting).color(ink::LIGHT));
                }
            });
        }
        match done {
            Some(Done::Answer(answer)) => answer_card(world, front, answer),
            Some(Done::Block) => block(world, front.other.clone()),
            None => {}
        }
        None
    }
}

/// A request card as the interface shows it, for a driver of the client.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ShownRequestCard {
    pub says: String,
    /// The player who asked.
    pub with: String,
    /// How long it still waits for its answer.
    pub left: Seconds,
    /// Whether it is the card in front, which the keys answer.
    pub front: bool,
}

pub fn shown(world: &World) -> Vec<ShownRequestCard> {
    let Some(now) = time::server_now(world) else {
        return Vec::new();
    };
    on_hud(world)
        .into_iter()
        .enumerate()
        .map(|(index, card)| ShownRequestCard {
            says: card.says,
            with: card.other.name,
            left: card.until.since(now),
            front: index == 0,
        })
        .collect()
}

/// Asks the player whether to block `player`, and blocks them once they confirm.
pub(crate) fn block(world: &mut World, player: SocietyMember) {
    let content = world.resource::<Content>().clone();
    let words = messages::of(&content);
    let name = &player.name;
    hud::confirm(
        world,
        Confirm {
            title: words.requests.block_title(name).into(),
            body: vec![words.requests.block_body(name).into()],
            confirm: words.requests.block().to_owned(),
            cancel: words.interface.cancel().to_owned(),
            answer: Box::new(move |world, blocked| {
                if blocked {
                    world.write_message(BlockCharacter {
                        account: player.account,
                        blocked: true,
                    });
                }
            }),
        },
    );
}

pub(crate) fn plugin(app: &mut App) {
    app.add_systems(Update, answer_keys.run_if(in_state(ClientScreen::Area)));
}

/// How long is left until `until`, as a card says it.
pub(crate) fn time_left(time: &TimeMessages, until: UnixMillis, now: UnixMillis) -> String {
    let left = until.since(now);
    match left.0.max(0.0) as u64 {
        0..3600 => time.countdown(left),
        _ => time.rough(left),
    }
}

enum Done {
    Answer(RequestAnswer),
    Block,
}

/// The cards the HUD shows now, the oldest first: the requests the player is asked.
fn on_hud(world: &World) -> Vec<RequestCard> {
    let Some(now) = time::server_now(world) else {
        return Vec::new();
    };
    session::my_character(world)
        .and_then(|me| me.get::<SocietyView>())
        .map(|view| {
            view.requests
                .iter()
                .filter(|card| !card.mine && now < card.shown_until)
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

fn front_card(
    ui: &mut Ui,
    world: &World,
    words: &RequestMessages,
    time: &TimeMessages,
    card: &RequestCard,
    now: UnixMillis,
) -> Option<Done> {
    let keys = world.resource::<InputMap>();
    let keyed = keys_answer(world);
    let cap = |action| keyed.then(|| keys.cap(action)).flatten();
    let mut done = None;
    ui.horizontal(|ui| {
        ui.add(egui::Label::new(face::body(&card.says)).wrap());
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            ui.label(face::small_faint(time_left(time, card.until, now)));
        });
    });
    ui.horizontal(|ui| {
        let accept = cap(InputAction::AcceptRequest);
        if controls::keyed(ui, words.accept(), accept.as_ref(), true).clicked() {
            done = Some(Done::Answer(RequestAnswer::Accept));
        }
        let deny = cap(InputAction::DenyRequest);
        if controls::keyed(ui, words.deny(), deny.as_ref(), true).clicked() {
            done = Some(Done::Answer(RequestAnswer::Deny));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if controls::button(ui, words.block()).clicked() {
                done = Some(Done::Block);
            }
        });
    });
    done
}

fn answer_card(world: &mut World, card: &RequestCard, answer: RequestAnswer) {
    world.write_message(AnswerSocialRequest {
        request: card.id,
        answer,
    });
}

/// Whether the keys answer the card in front: not while a call to an instance is up, which they
/// answer first, nor where they aren't read.
fn keys_answer(world: &World) -> bool {
    !area_instance::gather::card_up(world)
        && scope::reads(world, InputAction::AcceptRequest)
        && scope::reads(world, InputAction::DenyRequest)
}

fn answer_keys(world: &mut World) {
    if !keys_answer(world) {
        return;
    }
    let answer = if map::just_pressed(world, InputAction::AcceptRequest) {
        RequestAnswer::Accept
    } else if map::just_pressed(world, InputAction::DenyRequest) {
        RequestAnswer::Deny
    } else {
        return;
    };
    if let Some(front) = on_hud(world).first() {
        answer_card(world, front, answer);
    }
}

const WIDTH: f32 = 380.0;
const CARD_GAP: f32 = 6.0;
