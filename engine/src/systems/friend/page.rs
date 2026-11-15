//! The Friends page of the Social tab: finding a player, the requests in and out, the players
//! kept away, and the list of friends with what can be done with the one chosen.

use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ui::egui::{self, Align, Layout, Sense, Ui};
use ui::{controls, face, form, ink, input, sheet, words};

use super::search::{self, FindPlayers, PlayersFound};
use super::{AskFriendship, RemoveFriend};
use crate::core::content::Content;
use crate::core::time::{self, UnixMillis};
use crate::systems::GameSettings;
use crate::systems::hud::{self, Confirm};
use crate::systems::input::map::InputAction;
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;
use crate::systems::social_request::card as request_card;
use crate::systems::social_request::{
    AnswerSocialRequest, BlockPlayer, RequestAnswer, SocialRequestId, SocialRequestKind,
};
use crate::systems::society::tab::{self, SocialPage};
use crate::systems::society::{AccountNumber, FriendRow, RequestCard, SocietyMember, SocietyView};

pub struct FriendsPage;

impl SocialPage for FriendsPage {
    fn title(&self) -> &'static str {
        "Friends"
    }

    fn on(&self, world: &World) -> bool {
        world.resource::<GameSettings>().friends.is_some()
    }

    fn waiting(&self, world: &World) -> u32 {
        view(world).map_or(0, |view| asked_of_me(&view).count() as u32)
    }

    fn hints(&self, _: &World) -> Vec<(InputAction, &'static str)> {
        vec![(InputAction::Select, "choose")]
    }

    fn show(&self, ui: &mut Ui, world: &mut World) {
        let Some(view) = view(world) else {
            return;
        };
        hear(world);
        let room = ui.available_rect_before_wrap();
        let ([beside], list) = sheet::columns(ui, room, [BESIDE_WIDTH]);
        let mut done = None;
        let area = egui::ScrollArea::vertical().id_salt("friends beside");
        sheet::scroll_pane(ui, beside, area, |ui| {
            done = done.take().or(find(ui, world, &view));
            if world.resource::<Finding>().typed.trim().is_empty() {
                done = done.take().or(waiting(ui, &view));
            }
        });
        let area = egui::ScrollArea::vertical().id_salt("friends list");
        sheet::scroll_pane(ui, list, area, |ui| {
            done = done.take().or(friends(ui, world, &view));
        });
        ui.allocate_rect(room, Sense::hover());
        if let Some(done) = done {
            done.apply(world);
        }
    }
}

/// What the Friends page shows, for a driver of the client.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
pub struct ShownFriends {
    pub friends: Vec<ShownFriend>,
    /// Who asks the player to be their friend.
    pub asking: Vec<String>,
    /// Who the player asked and waits for.
    pub asked: Vec<String>,
    pub blocked: Vec<String>,
    /// What the search found for what is typed, once the server has answered.
    pub found: Option<Vec<FoundPlayer>>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ShownFriend {
    pub name: String,
    /// The area they play in, while they do.
    pub online_in: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct FoundPlayer {
    pub name: String,
    pub standing: PlayerStanding,
}

/// What a player found by name is to the player who searched.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlayerStanding {
    You,
    Friend,
    /// They have a request for friendship from the player, waiting.
    Asked,
    /// They ask the player to be their friend.
    Asking,
    Blocked,
    Stranger,
}

pub fn shown(world: &World) -> ShownFriends {
    let Some(view) = view(world) else {
        return ShownFriends::default();
    };
    let content = world.resource::<Content>();
    let names =
        |cards: Vec<&RequestCard>| cards.iter().map(|card| card.other.name.clone()).collect();
    let finding = world.resource::<Finding>();
    ShownFriends {
        friends: view
            .friends
            .iter()
            .map(|friend| ShownFriend {
                name: friend.player.name.clone(),
                online_in: friend.online_in.map(|area| area.get(content).name.clone()),
            })
            .collect(),
        asking: names(asked_of_me(&view).collect()),
        asked: names(asked_by_me(&view).collect()),
        blocked: view
            .blocked
            .iter()
            .map(|player| player.name.clone())
            .collect(),
        found: finding.answer().map(|found| {
            found
                .players
                .iter()
                .map(|player| FoundPlayer {
                    name: player.name.clone(),
                    standing: standing(&view, player).0,
                })
                .collect()
        }),
    }
}

pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<Finding>()
        .add_systems(OnExit(ClientScreen::Area), forget);
}

/// The search as the player has it: what is typed, what the server was asked and what it found.
#[derive(Resource, Default)]
struct Finding {
    typed: String,
    /// When what is typed last changed, on the interface's clock.
    typed_at: f64,
    asked: Option<String>,
    found: Option<PlayersFound>,
    chosen: Option<AccountNumber>,
}

impl Finding {
    /// What the server found for what is typed now.
    fn answer(&self) -> Option<&PlayersFound> {
        self.found
            .as_ref()
            .filter(|found| found.starts == self.typed)
    }
}

fn forget(mut finding: ResMut<Finding>) {
    *finding = Finding::default();
}

enum Done {
    Ask(AccountNumber),
    Answer(SocialRequestId, RequestAnswer),
    Unblock(AccountNumber),
    Block(SocietyMember),
    Remove(SocietyMember),
    Choose(AccountNumber),
    Act(&'static dyn tab::PlayerAction, SocietyMember),
}

impl Done {
    fn apply(self, world: &mut World) {
        match self {
            Done::Ask(account) => {
                world.write_message(AskFriendship(account));
            }
            Done::Answer(request, answer) => {
                world.write_message(AnswerSocialRequest { request, answer });
            }
            Done::Unblock(account) => {
                world.write_message(BlockPlayer {
                    account,
                    blocked: false,
                });
            }
            Done::Block(player) => request_card::block(world, player),
            Done::Remove(player) => remove(world, player),
            Done::Choose(account) => world.resource_mut::<Finding>().chosen = Some(account),
            Done::Act(action, player) => action.act(world, &player),
        }
    }
}

fn view(world: &World) -> Option<SocietyView> {
    session::my_character(world)?.get::<SocietyView>().cloned()
}

fn friendship(view: &SocietyView) -> impl Iterator<Item = &RequestCard> {
    view.requests
        .iter()
        .filter(|card| card.kind == SocialRequestKind::Friendship)
}

fn asked_of_me(view: &SocietyView) -> impl Iterator<Item = &RequestCard> {
    friendship(view).filter(|card| !card.mine)
}

fn asked_by_me(view: &SocietyView) -> impl Iterator<Item = &RequestCard> {
    friendship(view).filter(|card| card.mine)
}

/// What `player` is to the player `view` is of, and the request between the two that waits.
fn standing(
    view: &SocietyView,
    player: &SocietyMember,
) -> (PlayerStanding, Option<SocialRequestId>) {
    let is = |other: &SocietyMember| other.account == player.account;
    if view.account == player.account {
        return (PlayerStanding::You, None);
    }
    if view.blocked.iter().any(is) {
        return (PlayerStanding::Blocked, None);
    }
    if view.friends.iter().any(|friend| is(&friend.player)) {
        return (PlayerStanding::Friend, None);
    }
    if let Some(card) = asked_of_me(view).find(|card| is(&card.other)) {
        return (PlayerStanding::Asking, Some(card.id));
    }
    if asked_by_me(view).any(|card| is(&card.other)) {
        return (PlayerStanding::Asked, None);
    }
    (PlayerStanding::Stranger, None)
}

/// Takes what the server found, and asks it for what is typed once the typing has paused.
fn hear(world: &mut World) {
    let found = world
        .resource_mut::<Messages<PlayersFound>>()
        .drain()
        .last();
    let now = world.resource::<Time<Real>>().elapsed_secs_f64();
    let mut finding = world.resource_mut::<Finding>();
    if let Some(found) = found {
        finding.found = Some(found);
    }
    let paused = now - finding.typed_at >= TYPING_PAUSE;
    let unasked = finding.asked.as_deref() != Some(finding.typed.as_str());
    if paused && unasked && search::searches(&finding.typed) {
        let typed = finding.typed.clone();
        finding.asked = Some(typed.clone());
        world.write_message(FindPlayers(typed));
    }
}

fn find(ui: &mut Ui, world: &mut World, view: &SocietyView) -> Option<Done> {
    sheet::section(ui, "Find a player");
    let now = world.resource::<Time<Real>>().elapsed_secs_f64();
    let mut finding = world.resource_mut::<Finding>();
    let mut typed = finding.typed.clone();
    input::search(ui, &mut typed, "A name, two characters or more", None);
    if typed != finding.typed {
        finding.typed = typed;
        finding.typed_at = now;
    }
    let finding = world.resource::<Finding>();
    if finding.typed.trim().is_empty() {
        return None;
    }
    ui.add_space(GAP);
    if !search::searches(&finding.typed) {
        caption(ui, "Two characters or more");
        return None;
    }
    let Some(found) = finding.answer() else {
        caption(ui, "Looking");
        ui.ctx().request_repaint();
        return None;
    };
    if found.players.is_empty() {
        caption(ui, "Nobody by that name");
        return None;
    }
    let mut done = None;
    for player in &found.players {
        ui.push_id(("found", player.account.0), |ui| {
            done = done.take().or(found_row(ui, view, player));
        });
    }
    if found.in_all as usize > found.players.len() {
        let shown = found.players.len();
        let in_all = words::counted(found.in_all, "players");
        caption(ui, &format!("{in_all} · the first {shown} are shown"));
    }
    done
}

fn found_row(ui: &mut Ui, view: &SocietyView, player: &SocietyMember) -> Option<Done> {
    let (standing, request) = standing(view, player);
    let name = &player.name;
    let mut done = None;
    line(
        ui,
        name,
        standing == PlayerStanding::You,
        |ui| match standing {
            PlayerStanding::You => {
                ui.label(face::small_faint("You"));
            }
            PlayerStanding::Friend => {
                controls::pill(ui, "Friend", controls::Pill::Verdigris);
            }
            PlayerStanding::Asked => {
                controls::pill(ui, "Request sent", controls::Pill::Ink);
            }
            PlayerStanding::Asking => {
                ui.label(face::small_faint("asked you"));
                if let Some(request) = request
                    && named(controls::primary(ui, "Accept"), "Accept", name).clicked()
                {
                    done = Some(Done::Answer(request, RequestAnswer::Accept));
                }
            }
            PlayerStanding::Blocked => {
                if named(controls::button(ui, "Unblock"), "Unblock", name).clicked() {
                    done = Some(Done::Unblock(player.account));
                }
            }
            PlayerStanding::Stranger => {
                if named(controls::button(ui, "Block"), "Block", name).clicked() {
                    done = Some(Done::Block(player.clone()));
                }
                if named(controls::primary(ui, "Add friend"), "Add friend", name).clicked() {
                    done = Some(Done::Ask(player.account));
                }
            }
        },
    );
    done
}

/// The requests in and out and the players kept away, each with the one thing to do about it.
fn waiting(ui: &mut Ui, view: &SocietyView) -> Option<Done> {
    let mut done = None;
    let asking: Vec<&RequestCard> = asked_of_me(view).collect();
    if !asking.is_empty() {
        ui.add_space(GAP);
        form::section_count(ui, "Requests", asking.len());
        for card in asking {
            let name = &card.other.name;
            ui.push_id(("asking", card.id.0), |ui| {
                line(ui, name, false, |ui| {
                    if named(controls::button(ui, "Deny"), "Deny", name).clicked() {
                        done = Some(Done::Answer(card.id, RequestAnswer::Deny));
                    }
                    if named(controls::primary(ui, "Accept"), "Accept", name).clicked() {
                        done = Some(Done::Answer(card.id, RequestAnswer::Accept));
                    }
                });
            });
        }
    }
    let asked: Vec<&RequestCard> = asked_by_me(view).collect();
    if !asked.is_empty() {
        ui.add_space(GAP);
        form::section_count(ui, "Sent", asked.len());
        for card in asked {
            let name = &card.other.name;
            ui.push_id(("asked", card.id.0), |ui| {
                line(ui, name, false, |ui| {
                    if named(controls::button(ui, "Withdraw"), "Withdraw", name).clicked() {
                        done = Some(Done::Answer(card.id, RequestAnswer::TakeBack));
                    }
                    ui.label(face::small_faint("waiting"));
                });
            });
        }
    }
    if !view.blocked.is_empty() {
        ui.add_space(GAP);
        form::section_count(ui, "Blocked", view.blocked.len());
        for player in &view.blocked {
            let name = &player.name;
            ui.push_id(("blocked", player.account.0), |ui| {
                line(ui, name, false, |ui| {
                    if named(controls::button(ui, "Unblock"), "Unblock", name).clicked() {
                        done = Some(Done::Unblock(player.account));
                    }
                });
            });
        }
    }
    done
}

fn friends(ui: &mut Ui, world: &mut World, view: &SocietyView) -> Option<Done> {
    let most = world
        .resource::<GameSettings>()
        .friends
        .map_or(0, |friends| friends.most);
    form::section_count(ui, "Friends", format!("{} of {most}", view.friends.len()));
    if view.friends.is_empty() {
        ui.add_space(GAP);
        ui.add(
            egui::Label::new(face::body(
                "Friends can write to each other and find each other in the world.",
            ))
            .wrap(),
        );
        caption(ui, "Find a player by name to ask them.");
        return None;
    }
    let now = time::server_now(world);
    let content = world.resource::<Content>().clone();
    let chosen = world.resource::<Finding>().chosen;
    let chosen = view
        .friends
        .iter()
        .find(|friend| Some(friend.player.account) == chosen)
        .or(view.friends.first());
    let mut done = None;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for friend in &view.friends {
            let selected =
                chosen.is_some_and(|chosen| chosen.player.account == friend.player.account);
            ui.push_id(("friend", friend.player.account.0), |ui| {
                if friend_row(ui, world, &content, friend, selected, now) {
                    done = Some(Done::Choose(friend.player.account));
                }
            });
        }
    });
    let chosen = chosen?;
    let name = &chosen.player.name;
    ui.add_space(GAP);
    ui.horizontal(|ui| {
        for action in tab::player_actions(world) {
            let label = action.label();
            if named(controls::button(ui, label), label, name).clicked() {
                done = Some(Done::Act(action, chosen.player.clone()));
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if named(controls::danger(ui, "Remove friend"), "Remove friend", name).clicked() {
                done = Some(Done::Remove(chosen.player.clone()));
            }
            if named(controls::button(ui, "Block"), "Block", name).clicked() {
                done = Some(Done::Block(chosen.player.clone()));
            }
        });
    });
    done
}

/// A friend's row. Whether the player chose it.
fn friend_row(
    ui: &mut Ui,
    world: &World,
    content: &Content,
    friend: &FriendRow,
    selected: bool,
    now: Option<UnixMillis>,
) -> bool {
    let online = friend.online_in.map(|area| area.get(content).name.as_str());
    let state = match (online, now) {
        (Some(area), _) => format!("Online · {area}"),
        (None, Some(now)) => {
            let seen = words::ago(now.since(friend.last_seen).0);
            format!("Offline · seen {seen}")
        }
        (None, None) => "Offline".to_owned(),
    };
    let look = controls::RowLook {
        selected,
        faint: online.is_none(),
        ..controls::RowLook::default()
    };
    let name = &friend.player.name;
    let shown = controls::row(ui, look, |ui| {
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(face::small(state.as_str()).color(ink::INK3));
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.add(egui::Label::new(face::bold(name.as_str())).truncate());
                });
            });
        });
    });
    let response = shown
        .response
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| {
        let said = format!("{name}, {state}");
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, said)
    });
    crate::systems::input::map::clicked(world, InputAction::Select, &response)
}

/// Asks the player whether to end the friendship with `player`, and ends it once they confirm.
fn remove(world: &mut World, player: SocietyMember) {
    let name = &player.name;
    hud::confirm(
        world,
        Confirm {
            title: format!("Remove {name} from your friends?"),
            body: vec![format!(
                "You come off {name}'s list too. {name} is not told."
            )],
            confirm: "Remove".to_owned(),
            cancel: "Keep".to_owned(),
            answer: Box::new(move |world, removed| {
                if removed {
                    world.write_message(RemoveFriend(player.account));
                }
            }),
        },
    );
}

/// A player's name with what can be done about them at the line's end, the last thing added
/// standing nearest the name.
fn line(ui: &mut Ui, name: &str, faint: bool, ends: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.set_min_height(LINE_HEIGHT);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ends(ui);
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let name = face::body(name);
                let name = if faint { name.color(ink::INK3) } else { name };
                ui.add(egui::Label::new(name).truncate());
            });
        });
    });
}

/// Names a button for what it does and to whom, so two alike in one list are told apart.
fn named(response: egui::Response, does: &str, name: &str) -> egui::Response {
    let enabled = response.enabled();
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, format!("{does} {name}"))
    });
    response
}

fn caption(ui: &mut Ui, text: &str) {
    form::caption(ui, text, ink::INK2);
}

const BESIDE_WIDTH: f32 = 320.0;
const GAP: f32 = 8.0;
const LINE_HEIGHT: f32 = 28.0;
/// How long the typing rests before the server is asked, so it is never asked on every key.
const TYPING_PAUSE: f64 = 0.3;
