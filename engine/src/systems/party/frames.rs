use bevy::prelude::*;
use ui::egui::{self, Color32, Pos2, Rect, Sense, Ui, Vec2};
use ui::face::{Face, TextRole};
use ui::glyph::Mark;
use ui::ink;
use ui::tracker::{self, INDENT, ROW_GAP, TrackerHeading, TrackerLine, TrackerTone, WIDTH};

use super::{PartyMemberRow, PartyShown};
use crate::core::content::Content;
use crate::core::time::{self, UnixMillis};
use crate::systems::hud::{self, HudWidget};
use crate::systems::messages::{self, Messages};
use crate::systems::player::{ClientId, PlayedBy, session};
use crate::systems::settings::UserSettings;
use crate::systems::social_request::card::time_left;
use crate::systems::society::SocietyView;
use crate::systems::stat::{self, Vital};

pub const PARTY_FRAMES: &str = "party";

pub struct PartyFramesPlugin;

impl Plugin for PartyFramesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Folded>();
    }
}

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
        let others: Vec<&PartyMemberRow> = party
            .members
            .iter()
            .filter(|member| member.character.account != view.account)
            .collect();
        let now = time::server_now(world);
        let folded = world.resource::<Folded>().0;
        let opacity = world.resource::<UserSettings>().tracker_backing();
        let content = world.resource::<Content>().clone();
        let messages = messages::of(&content);
        let words = &messages.party;
        let title = words.tracker_title_folded(others.len());
        let heading = if folded {
            TrackerHeading {
                title: &title,
                said: words.tracker_show(),
            }
        } else {
            TrackerHeading {
                title: words.title(),
                said: words.tracker_hide(),
            }
        };
        let shown = tracker::show(ui, heading, folded, opacity.0 / 100.0, |ui, at| {
            others.iter().fold(at.y, |foot, member| {
                frame(
                    ui,
                    world,
                    messages,
                    party,
                    member,
                    now,
                    Pos2::new(at.x, foot),
                )
            })
        });
        if shown.toggled {
            world.resource_mut::<Folded>().0 = !folded;
        }
        Some(shown.drawn)
    }
}

#[derive(Resource, Default)]
struct Folded(bool);

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

/// What `standing` says in words, to whoever reads the interface.
pub(super) fn said(messages: &Messages, standing: &Standing, now: Option<UnixMillis>) -> String {
    let words = &messages.party;
    match standing {
        Standing::Seen { health } => words.health_percent((health * 100.0).round()),
        Standing::Elsewhere(area) => area.clone(),
        Standing::Away(leaves_at) => match leaves_at.zip(now) {
            Some((leaves_at, now)) => words.away_leaves(time_left(&messages.time, leaves_at, now)),
            None => words.away_stays().to_owned(),
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
    messages: &Messages,
    party: &PartyShown,
    member: &PartyMemberRow,
    now: Option<UnixMillis>,
    at: Pos2,
) -> f32 {
    let standing = standing(world, member);
    let id = ui.id().with(("party member", member.character.account));
    let top = at.y + tracker::ENTRY_GAP;
    let rows = Pos2::new(at.x + INDENT, top);
    tracker::bullet(ui, Pos2::new(at.x, top));
    let leads = party.leader == member.character.account;
    let mut foot = TrackerLine {
        at: rows,
        width: WIDTH - INDENT,
        mark: leads.then_some((Mark::Crown, ink::GOLD2)),
        text: &member.character.name,
        font: Face::BookBold.id(TextRole::Body.size()),
        color: Color32::WHITE,
    }
    .letter(ui, id.with("name"))
    .bottom();
    let row = |ui: &mut Ui, foot: f32, text: &str, tone: TrackerTone, which: &str| {
        TrackerLine {
            at: Pos2::new(rows.x, foot + ROW_GAP),
            width: WIDTH - INDENT,
            mark: tone.mark().map(|mark| (mark, tone.color())),
            text,
            font: TextRole::Small.font_id(),
            color: tone.color(),
        }
        .letter(ui, id.with(which))
        .bottom()
    };
    foot = match &standing {
        Standing::Seen { health } => {
            health_over_the_world(ui, Pos2::new(rows.x, foot + BAR_GAP), *health).bottom()
        }
        Standing::Elsewhere(_) => row(
            ui,
            foot,
            &said(messages, &standing, now),
            TrackerTone::Plain,
            "at",
        ),
        Standing::Away(_) => row(
            ui,
            foot,
            &said(messages, &standing, now),
            TrackerTone::Timed,
            "at",
        ),
    };
    let said = messages.party.row_named(
        super::page::named(&messages.party, party, member),
        said(messages, &standing, now),
    );
    let entry = Rect::from_min_max(Pos2::new(at.x, top), Pos2::new(at.x + WIDTH, foot));
    ui.interact(entry, id, Sense::hover())
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, said.clone()));
    foot
}

/// A bar as full of health as `health` says, edged to read over whatever the world shows.
fn health_over_the_world(ui: &Ui, at: Pos2, health: f32) -> Rect {
    let rect = Rect::from_min_size(at, Vec2::new(BAR_WIDTH, BAR_HEIGHT));
    let painter = ui.painter();
    painter.rect_filled(rect.expand(1.5), BAR_HEIGHT, Color32::BLACK);
    painter.rect_filled(rect, BAR_HEIGHT / 2.0, Color32::from_gray(60));
    let full = Rect::from_min_size(rect.min, Vec2::new(rect.width() * health, rect.height()));
    painter.rect_filled(full, BAR_HEIGHT / 2.0, ink::VERMILION);
    rect
}

const BAR_WIDTH: f32 = 120.0;
const BAR_HEIGHT: f32 = 6.0;
const BAR_GAP: f32 = 5.0;
const FROM_TOP: f32 = 96.0;
