//! The request cards: a piece of the HUD that shows a player the requests they are asked, the
//! oldest in front, and takes their answer. What a player asked of others shows only on the
//! pages that list it.

use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ui::egui::{self, Align, Layout, Pos2, Rect, Ui};
use ui::{controls, dialog, face, ink};

use super::{AnswerSocialRequest, BlockPlayer, RequestAnswer};
use crate::core::time::{self, UnixMillis};
use crate::core::units::Seconds;
use crate::systems::hud::{self, Confirm, HudWidget};
use crate::systems::input::map::{self, InputAction, InputMap};
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
        let (front, behind) = cards.split_first()?;
        ui.spacing_mut().item_spacing.y = CARD_GAP;
        let done = dialog::note(ui, WIDTH, |ui| front_card(ui, world, front, now));
        if let Some(next) = behind.first() {
            ui.with_layout(Layout::top_down(Align::Max), |ui| {
                let left = time_left(next.until, now);
                ui.label(face::small(format!("{} · {left}", next.says)).color(ink::LIGHT));
                if behind.len() > 1 {
                    let waiting = format!("{} more waiting", behind.len() - 1);
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
    let name = &player.name;
    hud::confirm(
        world,
        Confirm {
            title: format!("Block {name}?").into(),
            body: vec![
                format!(
                    "{name} will not be able to send you requests or messages, and you will not \
                     see what they say in chat. {name} is not told."
                )
                .into(),
            ],
            confirm: "Block".to_owned(),
            cancel: "Cancel".to_owned(),
            answer: Box::new(move |world, blocked| {
                if blocked {
                    world.write_message(BlockPlayer {
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
pub(crate) fn time_left(until: UnixMillis, now: UnixMillis) -> String {
    let seconds = until.since(now).0.max(0.0) as u64;
    match seconds {
        0..3600 => format!("{}:{:02}", seconds / 60, seconds % 60),
        3600..172_800 => format!("{} h", seconds / 3600),
        _ => format!("{} d", seconds / 86_400),
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

fn front_card(ui: &mut Ui, world: &World, card: &RequestCard, now: UnixMillis) -> Option<Done> {
    let keys = world.resource::<InputMap>();
    let mut done = None;
    ui.horizontal(|ui| {
        ui.add(egui::Label::new(face::body(&card.says)).wrap());
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            ui.label(face::small_faint(time_left(card.until, now)));
        });
    });
    ui.horizontal(|ui| {
        let accept = keys.name(InputAction::AcceptRequest);
        if controls::keyed(ui, "Accept", accept.as_deref(), true).clicked() {
            done = Some(Done::Answer(RequestAnswer::Accept));
        }
        let deny = keys.name(InputAction::DenyRequest);
        if controls::keyed(ui, "Deny", deny.as_deref(), true).clicked() {
            done = Some(Done::Answer(RequestAnswer::Deny));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if controls::button(ui, "Block").clicked() {
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

/// The keys answer the card in front.
fn answer_keys(world: &mut World) {
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
