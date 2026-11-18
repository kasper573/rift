//! What players are told of saving: what the game changing did to the character they come back
//! to, once, and that the game cannot save for as long as it can't.

use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ui::egui::{self, Align, Align2, Id, Layout, Vec2};
use ui::{controls, dialog, dusk, face, float, ink};

use super::{GameChange, ServerSaving};
use crate::core::content::Content;
use crate::core::gui::{self, Band};
use crate::core::text::LineText;
use crate::core::time::{LocalClock, UnixMillis};
use crate::systems::history::{self, HistoryEntry, HistoryTopic};
use crate::systems::player::{ClientId, Players};
use crate::systems::screen::ClientScreen;
use crate::systems::{GameSettings, messages, visibility};

pub fn register(app: &mut App) {
    use bevy_replication::AppMessageExt;
    app.add_server_message::<GameChanged>()
        .add_server_message::<SavingNews>();
}

/// What the game changing did to the character a connection's player comes back to, until that
/// character is in the world for them to be told.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct Returned {
    pub changes: Vec<GameChange>,
}

/// What the game changing did to a character, in the game's words.
#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct GameChanged {
    pub lines: Vec<ChangeLine>,
}

/// One thing that changed, and what became of it.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
pub struct ChangeLine {
    pub what: String,
    pub became: String,
}

/// How saving stands, as every client is told when it changes.
#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct SavingNews(pub ServerSaving);

/// Tells each player who came back to a game that changed what became of their character, once
/// it is in the world, and writes it in their history.
pub fn tell_returned(world: &mut World, returned: &mut QueryState<(Entity, &ClientId, &Returned)>) {
    let players = world.resource::<Players>();
    let back: Vec<(Entity, Entity, Vec<GameChange>)> = returned
        .iter(world)
        .filter_map(|(conn, client, returned)| {
            let character = *players.0.get(client)?;
            Some((conn, character, returned.changes.clone()))
        })
        .collect();
    for (conn, character, changes) in back {
        world.entity_mut(conn).remove::<Returned>();
        let content = world.resource::<Content>().clone();
        let messages = messages::of(&content);
        let heading = messages.save.game_changed().to_owned();
        let lines: Vec<ChangeLine> = changes.iter().map(|change| change.line(&content)).collect();
        for line in &lines {
            let text = LineText::plain(messages.save.change_entry(&line.what, &line.became));
            let entry = HistoryEntry::of(HistoryTopic::Notification, text);
            history::record(world, character, entry.by(Some(heading.clone())));
        }
        visibility::send_to(world, conn, GameChanged { lines });
    }
}

/// Tells every client how saving stands as it changes, and a client that connects while nothing
/// is being saved.
pub fn tell_saving(
    world: &mut World,
    clients: &mut QueryState<Entity, With<ClientId>>,
    arrivals: &mut QueryState<Entity, Added<ClientId>>,
    mut told: Local<ServerSaving>,
) {
    let stands = world
        .get_resource::<ServerSaving>()
        .copied()
        .unwrap_or_default();
    let arrived: Vec<Entity> = arrivals.iter(world).collect();
    let owed = if stands != *told {
        clients.iter(world).collect()
    } else if stands.failing_since.is_some() {
        arrived
    } else {
        Vec::new()
    };
    *told = stands;
    for conn in owed {
        visibility::send_to(world, conn, SavingNews(stands));
    }
}

impl GameChange {
    /// The change, in the words of the game `content` is of.
    pub fn line(&self, content: &Content) -> ChangeLine {
        let messages = messages::of(content);
        match self {
            GameChange::Gone { table, name } => ChangeLine {
                what: messages.save.gone(table, name),
                became: messages.save.kept_aside().to_owned(),
            },
            GameChange::DidNotFit { slots, stacks } => ChangeLine {
                what: messages.save.stacks_did_not_fit(*slots, *stacks),
                became: match content.built::<GameSettings>().mail {
                    Some(_) => messages.mail.sent_in_a_letter().to_owned(),
                    None => messages.save.kept_aside().to_owned(),
                },
            },
            GameChange::SkillsReset { class, points } => ChangeLine {
                what: messages.save.skills_rearranged(class),
                became: messages.save.points_returned(*points),
            },
        }
    }
}

/// Shows a player what `tell_returned` and `tell_saving` tell their client.
pub struct SavingNoticePlugin;

impl Plugin for SavingNoticePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Told>()
            .add_systems(Update, hear.run_if(resource_exists::<GameSettings>))
            .add_systems(OnExit(ClientScreen::Area), forget)
            .add_systems(
                EguiPrimaryContextPass,
                (
                    draw_notice.in_set(Band::Notices),
                    draw_card.in_set(Band::Modal),
                )
                    .run_if(in_state(ClientScreen::Area)),
            );
    }
}

/// What the interface shows of saving, for a driver of the client.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
pub struct ShownSaving {
    /// The notice up, by its title: that the game cannot save, or that it saves again.
    pub notice: Option<String>,
    /// The card that tells what the game changing did to the character, while it is up.
    pub card: Vec<ChangeLine>,
}

pub fn shown(world: &World) -> ShownSaving {
    let told = world.resource::<Told>();
    let messages = messages::of(world.resource::<Content>());
    let now = gui::context(world).map(|gui| gui.input(|input| input.time));
    ShownSaving {
        notice: match (told.failing_since, told.saved_until, now) {
            (Some(_), ..) => Some(messages.save.not_saving().to_owned()),
            (None, Some(until), Some(now)) if now < until => Some(messages.save.saved().to_owned()),
            _ => None,
        },
        card: told.changed.clone().unwrap_or_default(),
    }
}

#[derive(Resource, Default)]
struct Told {
    failing_since: Option<UnixMillis>,
    /// Until when, on the interface's clock, it says that the game saves again.
    saved_until: Option<f64>,
    changed: Option<Vec<ChangeLine>>,
}

fn hear(world: &mut World) {
    let saving: Vec<SavingNews> = world
        .resource_mut::<Messages<SavingNews>>()
        .drain()
        .collect();
    let changed: Vec<GameChanged> = world
        .resource_mut::<Messages<GameChanged>>()
        .drain()
        .collect();
    let now = gui::context(world).map(|gui| gui.input(|input| input.time));
    let content = world.resource::<Content>();
    let messages = &messages::of(content).save;
    let words = [messages.saved(), messages.saved_again()]
        .iter()
        .map(|said| said.split_whitespace().count())
        .sum();
    let stays = content.built::<GameSettings>().reading.of(words);
    let mut told = world.resource_mut::<Told>();
    for SavingNews(stands) in saving {
        if told.failing_since.is_some() && stands.failing_since.is_none() {
            told.saved_until = now.map(|now| now + f64::from(stays.0));
        }
        told.failing_since = stands.failing_since;
    }
    if let Some(changed) = changed.into_iter().next_back() {
        told.changed = Some(changed.lines);
    }
}

fn forget(mut told: ResMut<Told>) {
    *told = Told::default();
}

fn draw_notice(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let told = world.resource::<Told>();
    let messages = &messages::of(world.resource::<Content>()).save;
    let (title, color, lines) = match (told.failing_since, told.saved_until) {
        (Some(since), _) => {
            let since = LocalClock::of_this_machine().label(since);
            let at_stake = messages.not_saving_since(since);
            (
                messages.not_saving(),
                dusk::NOTICE,
                vec![at_stake, messages.play_on().to_owned()],
            )
        }
        (None, Some(until)) if gui.input(|input| input.time) < until => (
            messages.saved(),
            dusk::CHARACTER,
            vec![messages.saved_again().to_owned()],
        ),
        _ => return,
    };
    let area = egui::Area::new(Id::new("saving"))
        .order(Band::Notices.order())
        .anchor(Align2::LEFT_TOP, Vec2::splat(NOTICE_FROM_CORNER))
        .interactable(false);
    float::anchored(&gui, area, |ui| {
        dusk::frame().show(ui, |ui| {
            ui.set_max_width(NOTICE_WIDTH);
            ui.label(face::bold(title).color(color));
            for line in &lines {
                ui.add(egui::Label::new(face::body(line).color(dusk::TEXT)).wrap());
            }
        });
    });
    gui.request_repaint_after(std::time::Duration::from_secs(1));
}

fn draw_card(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let Some(lines) = world.resource::<Told>().changed.clone() else {
        return;
    };
    let messages = &messages::of(world.resource::<Content>()).save;
    let mut carried_on = false;
    let modal = egui::Modal::new(Id::new("game-changed"))
        .backdrop_color(ink::ink_wash(0.3))
        .frame(egui::Frame::NONE)
        .show(&gui, |ui| {
            dialog::note(ui, CARD_WIDTH, |ui| {
                dialog::title(ui, messages.game_changed());
                for line in &lines {
                    ui.add(egui::Label::new(face::body(&line.what)).wrap());
                    ui.add(egui::Label::new(face::small_faint(&line.became)).wrap());
                    ui.add_space(LINE_GAP);
                }
                ui.with_layout(Layout::right_to_left(Align::Max), |ui| {
                    let carry_on = controls::primary(ui, messages.carry_on());
                    carried_on = carry_on.clicked() || dialog::keys(ui).confirm;
                });
            });
        });
    if carried_on || modal.should_close() {
        world.resource_mut::<Told>().changed = None;
    }
}

const NOTICE_FROM_CORNER: f32 = 16.0;
const NOTICE_WIDTH: f32 = 380.0;
const CARD_WIDTH: f32 = 520.0;
const LINE_GAP: f32 = 8.0;
