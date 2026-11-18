//! Who a player takes requests from, and who they keep away: the one part of the settings that
//! is their account's and the server's to keep, not their device's.

use strum::VariantArray;
use ui::egui::{self, Ui};
use ui::float::Hinted;
use ui::{controls, form, ink};

use super::{AcceptsRequestsFrom, BlockCharacter, ChooseAcceptsRequests, RequestMessages};
use crate::systems::society::{AccountNumber, SocietyView};

/// What the player chose in the section.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SocialChoice {
    Accepts(AcceptsRequestsFrom),
    Unblock(AccountNumber),
}

impl SocialChoice {
    /// Sends the choice to the server.
    pub(crate) fn send(self, world: &mut bevy::prelude::World) {
        match self {
            SocialChoice::Accepts(from) => {
                world.write_message(ChooseAcceptsRequests(from));
            }
            SocialChoice::Unblock(account) => {
                world.write_message(BlockCharacter {
                    account,
                    blocked: false,
                });
            }
        }
    }
}

/// The Social section of the settings, for an account that stands in society as `view` shows.
/// The players it keeps away are listed with `blocked`, for a game that lists them nowhere else.
pub(crate) fn section(
    ui: &mut Ui,
    words: &RequestMessages,
    view: &SocietyView,
    blocked: bool,
) -> Option<SocialChoice> {
    let mut choice = None;
    controls::form(ui, ("settings", "social"), |ui| {
        let label = ui.label(words.accept_from());
        let mut picked = view.accepts;
        controls::combo("accept requests from")
            .selected_text(name(words, view.accepts))
            .show_ui(ui, |ui| {
                for &from in AcceptsRequestsFrom::VARIANTS {
                    ui.selectable_value(&mut picked, from, name(words, from))
                        .hinted(admits(words, from));
                }
            })
            .response
            .labelled_by(label.id);
        if picked != view.accepts {
            choice = Some(SocialChoice::Accepts(picked));
        }
        ui.end_row();
    });
    form::caption(ui, admits(words, view.accepts), ink::INK2);
    form::caption(ui, words.kept_with_account(), ink::INK2);
    if blocked && !view.blocked.is_empty() {
        ui.add_space(6.0);
        form::section_count(ui, words.blocked_heading(), view.blocked.len());
        for player in &view.blocked {
            ui.horizontal(|ui| {
                ui.label(&player.name);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if controls::button(ui, words.unblock()).clicked() {
                        choice = Some(SocialChoice::Unblock(player.account));
                    }
                });
            });
        }
    }
    choice
}

fn name(words: &RequestMessages, from: AcceptsRequestsFrom) -> &str {
    match from {
        AcceptsRequestsFrom::Everyone => words.from_everyone(),
        AcceptsRequestsFrom::Friends => words.from_friends(),
        AcceptsRequestsFrom::Party => words.from_party(),
        AcceptsRequestsFrom::Nobody => words.from_nobody(),
    }
}

/// Who may send a request to a player who chose `from`.
fn admits(words: &RequestMessages, from: AcceptsRequestsFrom) -> &str {
    match from {
        AcceptsRequestsFrom::Everyone => words.admits_everyone(),
        AcceptsRequestsFrom::Friends => words.admits_friends(),
        AcceptsRequestsFrom::Party => words.admits_party(),
        AcceptsRequestsFrom::Nobody => words.admits_nobody(),
    }
}
