//! The party frames: a piece of the HUD that shows the other members of the player's party,
//! each with their health while they are in sight and where they are while they are not.

use bevy::prelude::*;
use ui::egui::{self, Align, Layout, Pos2, Rect, Sense, Ui, Vec2};
use ui::{controls, dialog, face, ink};

use super::{PartyMemberRow, PartyShown};
use crate::core::content::Content;
use crate::core::time::{self, UnixMillis};
use crate::systems::hud::{self, HudWidget};
use crate::systems::mount::{Riding, SeatIndex};
use crate::systems::player::{ClientId, PlayedBy, session};
use crate::systems::social_request::card::time_left;
use crate::systems::society::SocietyView;
use crate::systems::stat::{self, Vital};

pub const PARTY_FRAMES: &str = "party";

pub struct PartyFramesWidget;

impl HudWidget for PartyFramesWidget {
    fn fallback(&self, _: &World, screen: Rect) -> Pos2 {
        screen.left_top() + Vec2::new(hud::MARGIN, FROM_TOP)
    }

    fn hidden(&self, world: &World) -> bool {
        party(world).is_none_or(|party| party.members.len() < 2)
    }

    fn show(&self, ui: &mut Ui, world: &mut World) -> Option<Rect> {
        let view = session::my_character(world)?.get::<SocietyView>()?.clone();
        let party = view.party.as_ref()?;
        let now = time::server_now(world);
        dialog::note(ui, WIDTH, |ui| {
            ui.label(face::small("Party").color(ink::INK3));
            for member in &party.members {
                if member.player.account != view.account {
                    frame(ui, world, party, member, now);
                }
            }
        });
        None
    }
}

/// How a member of the party stands, as the frames and the Party page say it.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Standing {
    /// In sight: how much of their health they hold, from none to all of it.
    Seen { health: f32 },
    /// Playing out of sight, in the area called this.
    Elsewhere(String),
    /// Their player is gone, and they leave the party then unless they are back.
    Away(Option<UnixMillis>),
}

pub(super) fn standing(world: &mut World, member: &PartyMemberRow) -> Standing {
    let Some(area) = member.area else {
        return Standing::Away(member.leaves_at);
    };
    let seen = member.client.and_then(|client| character_of(world, client));
    match seen {
        Some(character) => {
            let most = stat::most(world, character, Vital::Health);
            let now = stat::current(world, character, Vital::Health);
            Standing::Seen {
                health: if most > 0.0 {
                    (now / most).clamp(0.0, 1.0)
                } else {
                    0.0
                },
            }
        }
        None => Standing::Elsewhere(area.get(world.resource::<Content>()).name.clone()),
    }
}

/// The character `client` plays, while it is in sight.
pub(super) fn character_of(world: &mut World, client: ClientId) -> Option<Entity> {
    world
        .query::<(Entity, &PlayedBy)>()
        .iter(world)
        .find(|(_, by)| by.client == client)
        .map(|(character, _)| character)
}

/// A bar as full of health as `health` says, from none to all.
pub(super) fn health_bar(ui: &mut Ui, health: f32, width: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, BAR_HEIGHT), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, BAR_HEIGHT / 2.0, ink::ink_wash(0.25));
    let full = Rect::from_min_size(rect.min, Vec2::new(rect.width() * health, rect.height()));
    painter.rect_filled(full, BAR_HEIGHT / 2.0, ink::WAX);
}

/// What `standing` says in words, to whoever reads the interface.
pub(super) fn said(standing: &Standing, now: Option<UnixMillis>) -> String {
    match standing {
        Standing::Seen { health } => format!("health {:.0}%", health * 100.0),
        Standing::Elsewhere(area) => area.clone(),
        Standing::Away(leaves_at) => match leaves_at.zip(now) {
            Some((leaves_at, now)) => {
                format!("Away · leaves the party in {}", time_left(leaves_at, now))
            }
            None => "Away".to_owned(),
        },
    }
}

fn party(world: &World) -> Option<&PartyShown> {
    session::my_character(world)?
        .get::<SocietyView>()?
        .party
        .as_ref()
}

fn frame(
    ui: &mut Ui,
    world: &mut World,
    party: &PartyShown,
    member: &PartyMemberRow,
    now: Option<UnixMillis>,
) {
    let standing = standing(world, member);
    let seat = shared_seat(world, member);
    let name = super::page::named(party, member);
    let row = ui.horizontal(|ui| {
        ui.set_width(WIDTH);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            match &standing {
                Standing::Seen { health } => health_bar(ui, *health, BAR_WIDTH),
                other => {
                    ui.label(face::small(said(other, now)).color(ink::INK3));
                }
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let name = face::body(name.as_str());
                let name = match standing {
                    Standing::Seen { .. } => name,
                    _ => name.color(ink::INK3),
                };
                ui.add(egui::Label::new(name).truncate());
                if let Some(seat) = seat {
                    controls::pill(ui, &format!("seat {}", seat.0 + 1), controls::Pill::Ink);
                }
            });
        });
    });
    let said = format!("{name}, {}", said(&standing, now));
    row.response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, said.clone()));
}

/// The seat `member` holds on the mount the player's character rides, where they ride together.
fn shared_seat(world: &mut World, member: &PartyMemberRow) -> Option<SeatIndex> {
    let mine = session::my_character(world)?.get::<Riding>()?.driver;
    let character = character_of(world, member.client?)?;
    world
        .get::<Riding>(character)
        .filter(|riding| riding.driver == mine)
        .map(|riding| riding.seat)
}

const WIDTH: f32 = 230.0;
const BAR_WIDTH: f32 = 110.0;
const BAR_HEIGHT: f32 = 6.0;
const FROM_TOP: f32 = 96.0;
