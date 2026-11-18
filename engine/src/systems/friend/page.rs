//! The Friends page of the Social tab: finding a player, the requests in and out, the players
//! kept away, and the list of friends with what can be done with the one chosen.

use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ui::egui::{self, Align, Layout, Sense, Ui};
use ui::face::TextRole;
use ui::{controls, face, form, ink, input, sheet};

use super::search::{self, CharactersFound, FindCharacters};
use super::{AskFriendship, RemoveFriend};
use crate::core::content::Content;
use crate::core::text::LineText;
use crate::core::text::{self, TextLook};
use crate::core::time::{self, UnixMillis};
use crate::systems::GameSettings;
use crate::systems::hud::{self, Confirm};
use crate::systems::input::map::InputAction;
use crate::systems::mention::Mention;
use crate::systems::messages;
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;
use crate::systems::social_request::card as request_card;
use crate::systems::social_request::{
    AnswerSocialRequest, BlockCharacter, RequestAnswer, SocialRequestId, SocialRequestKind,
};
use crate::systems::society::tab::{self, SocialPage};
use crate::systems::society::{AccountNumber, FriendRow, RequestCard, SocietyMember, SocietyView};

pub struct FriendsPage;

impl SocialPage for FriendsPage {
    fn key(&self) -> &'static str {
        "friends"
    }

    fn title<'a>(&self, messages: &'a messages::Messages) -> &'a str {
        messages.friends.title()
    }

    fn on(&self, world: &World) -> bool {
        world.resource::<GameSettings>().friends.is_some()
    }

    fn waiting(&self, world: &World) -> u32 {
        view(world).map_or(0, |view| asked_of_me(&view).count() as u32)
    }

    fn hints(&self, world: &World) -> Vec<(InputAction, String)> {
        let messages = messages::of(world.resource::<Content>());
        let listed = view(world).is_some_and(|view| !view.friends.is_empty());
        listed
            .then(|| {
                (
                    InputAction::Select,
                    messages.friends.hint_choose().to_owned(),
                )
            })
            .into_iter()
            .collect()
    }

    fn show(&self, ui: &mut Ui, world: &mut World) {
        let Some(view) = view(world) else {
            return;
        };
        hear(world);
        let content = world.resource::<Content>().clone();
        let words = messages::of(&content);
        let room = ui.available_rect_before_wrap();
        let ([beside], list) = sheet::columns(ui, room, [BESIDE_WIDTH]);
        let mut done = None;
        let area = egui::ScrollArea::vertical().id_salt("friends beside");
        sheet::scroll_pane(ui, beside, area, |ui| {
            done = done.take().or(find(ui, world, words, &view));
            if world.resource::<Finding>().typed.trim().is_empty() {
                done = done.take().or(waiting(ui, words, &view));
            }
        });
        let area = egui::ScrollArea::vertical().id_salt("friends list");
        sheet::scroll_pane(ui, list, area, |ui| {
            done = done.take().or(friends(ui, world, words, &view));
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
    pub found: Option<Vec<FoundCharacter>>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ShownFriend {
    pub name: String,
    /// The area they play in, while they do.
    pub online_in: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct FoundCharacter {
    pub name: String,
    pub standing: CharacterStanding,
}

/// What a player found by name is to the player who searched.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CharacterStanding {
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
                name: friend.character.name.clone(),
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
                .characters
                .iter()
                .map(|player| FoundCharacter {
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
    found: Option<CharactersFound>,
    chosen: Option<AccountNumber>,
}

impl Finding {
    /// What the server found for what is typed now.
    fn answer(&self) -> Option<&CharactersFound> {
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
    Act(&'static dyn tab::CharacterAction, SocietyMember),
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
                world.write_message(BlockCharacter {
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
) -> (CharacterStanding, Option<SocialRequestId>) {
    let is = |other: &SocietyMember| other.account == player.account;
    if view.account == player.account {
        return (CharacterStanding::You, None);
    }
    if view.blocked.iter().any(is) {
        return (CharacterStanding::Blocked, None);
    }
    if view.friends.iter().any(|friend| is(&friend.character)) {
        return (CharacterStanding::Friend, None);
    }
    if let Some(card) = asked_of_me(view).find(|card| is(&card.other)) {
        return (CharacterStanding::Asking, Some(card.id));
    }
    if asked_by_me(view).any(|card| is(&card.other)) {
        return (CharacterStanding::Asked, None);
    }
    (CharacterStanding::Stranger, None)
}

/// Takes what the server found, and asks it for what is typed once the typing has paused.
fn hear(world: &mut World) {
    let found = world
        .resource_mut::<Messages<CharactersFound>>()
        .drain()
        .last();
    let now = world.resource::<Time<Real>>().elapsed_secs_f64();
    let content = world.resource::<Content>().clone();
    let mut finding = world.resource_mut::<Finding>();
    let searches = search::searches(&content, &finding.typed);
    if let Some(found) = found {
        finding.found = Some(found);
    }
    let paused = now - finding.typed_at >= TYPING_PAUSE;
    let unasked = finding.asked.as_deref() != Some(finding.typed.as_str());
    if paused && unasked && searches {
        let typed = finding.typed.clone();
        finding.asked = Some(typed.clone());
        world.write_message(FindCharacters(typed));
    }
}

fn find(
    ui: &mut Ui,
    world: &mut World,
    words: &messages::Messages,
    view: &SocietyView,
) -> Option<Done> {
    let all = words;
    let words = &all.friends;
    sheet::section(ui, words.find_heading());
    let now = world.resource::<Time<Real>>().elapsed_secs_f64();
    let content = world.resource::<Content>().clone();
    let mut finding = world.resource_mut::<Finding>();
    let mut typed = finding.typed.clone();
    let hint = words.search_hint(search::least_letters(&content));
    input::search(ui, &mut typed, &hint, None);
    if typed != finding.typed {
        finding.typed = typed;
        finding.typed_at = now;
    }
    let finding = world.resource::<Finding>();
    if finding.typed.trim().is_empty() {
        return None;
    }
    ui.add_space(GAP);
    if !search::searches(&content, &finding.typed) {
        caption(ui, &words.type_more(search::least_letters(&content)));
        return None;
    }
    let Some(found) = finding.answer() else {
        caption(ui, words.looking());
        ui.ctx().request_repaint();
        return None;
    };
    if found.characters.is_empty() {
        caption(ui, words.nobody_found());
        return None;
    }
    let mut done = None;
    for player in &found.characters {
        ui.push_id(("found", player.account.0), |ui| {
            done = done.take().or(found_row(ui, all, view, player));
        });
    }
    if found.in_all as usize > found.characters.len() {
        let shown = found.characters.len();
        caption(ui, &words.found_shown(found.in_all, shown));
    }
    done
}

fn found_row(
    ui: &mut Ui,
    words: &messages::Messages,
    view: &SocietyView,
    player: &SocietyMember,
) -> Option<Done> {
    let (standing, request) = standing(view, player);
    let name = &player.name;
    let mut done = None;
    line(
        ui,
        name,
        standing == CharacterStanding::You,
        |ui| match standing {
            CharacterStanding::You => {
                ui.label(face::small_faint(words.friends.you()));
            }
            CharacterStanding::Friend => {
                controls::pill(ui, words.friends.friend(), controls::Pill::Verdigris);
            }
            CharacterStanding::Asked => {
                controls::pill(ui, words.friends.request_sent(), controls::Pill::Ink);
            }
            CharacterStanding::Asking => {
                ui.label(face::small_faint(words.friends.asked_you()));
                let accept = words.requests.accept();
                if let Some(request) = request
                    && named(words, controls::primary(ui, accept), accept, name).clicked()
                {
                    done = Some(Done::Answer(request, RequestAnswer::Accept));
                }
            }
            CharacterStanding::Blocked => {
                let unblock = words.requests.unblock();
                if named(words, controls::button(ui, unblock), unblock, name).clicked() {
                    done = Some(Done::Unblock(player.account));
                }
            }
            CharacterStanding::Stranger => {
                let block = words.requests.block();
                if named(words, controls::button(ui, block), block, name).clicked() {
                    done = Some(Done::Block(player.clone()));
                }
                let add = words.friends.add_friend();
                if named(words, controls::primary(ui, add), add, name).clicked() {
                    done = Some(Done::Ask(player.account));
                }
            }
        },
    );
    done
}

/// The requests in and out and the players kept away, each with the one thing to do about it.
fn waiting(ui: &mut Ui, words: &messages::Messages, view: &SocietyView) -> Option<Done> {
    let mut done = None;
    let asking: Vec<&RequestCard> = asked_of_me(view).collect();
    if !asking.is_empty() {
        ui.add_space(GAP);
        form::section_count(ui, words.friends.requests_heading(), asking.len());
        for card in asking {
            let name = &card.other.name;
            ui.push_id(("asking", card.id.0), |ui| {
                line(ui, name, false, |ui| {
                    let deny = words.requests.deny();
                    if named(words, controls::button(ui, deny), deny, name).clicked() {
                        done = Some(Done::Answer(card.id, RequestAnswer::Deny));
                    }
                    let accept = words.requests.accept();
                    if named(words, controls::primary(ui, accept), accept, name).clicked() {
                        done = Some(Done::Answer(card.id, RequestAnswer::Accept));
                    }
                });
            });
        }
    }
    let asked: Vec<&RequestCard> = asked_by_me(view).collect();
    if !asked.is_empty() {
        ui.add_space(GAP);
        form::section_count(ui, words.friends.sent_heading(), asked.len());
        for card in asked {
            let name = &card.other.name;
            ui.push_id(("asked", card.id.0), |ui| {
                line(ui, name, false, |ui| {
                    let withdraw = words.friends.withdraw();
                    if named(words, controls::button(ui, withdraw), withdraw, name).clicked() {
                        done = Some(Done::Answer(card.id, RequestAnswer::TakeBack));
                    }
                    ui.label(face::small_faint(words.friends.waiting()));
                });
            });
        }
    }
    if !view.blocked.is_empty() {
        ui.add_space(GAP);
        form::section_count(ui, words.requests.blocked_heading(), view.blocked.len());
        for player in &view.blocked {
            let name = &player.name;
            ui.push_id(("blocked", player.account.0), |ui| {
                line(ui, name, false, |ui| {
                    let unblock = words.requests.unblock();
                    if named(words, controls::button(ui, unblock), unblock, name).clicked() {
                        done = Some(Done::Unblock(player.account));
                    }
                });
            });
        }
    }
    done
}

fn friends(
    ui: &mut Ui,
    world: &mut World,
    words: &messages::Messages,
    view: &SocietyView,
) -> Option<Done> {
    let most = world
        .resource::<GameSettings>()
        .friends
        .map_or(0, |friends| friends.most);
    let count = words.friends.friends_of_most(view.friends.len(), most);
    form::section_count(ui, words.friends.friends_heading(), count);
    if view.friends.is_empty() {
        ui.add_space(GAP);
        ui.add(egui::Label::new(face::body(words.friends.friends_intro())).wrap());
        caption(ui, words.friends.find_to_ask());
        return None;
    }
    let now = time::server_now(world);
    let content = world.resource::<Content>().clone();
    let chosen = world.resource::<Finding>().chosen;
    let chosen = view
        .friends
        .iter()
        .find(|friend| Some(friend.character.account) == chosen)
        .or(view.friends.first());
    let mut done = None;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for friend in &view.friends {
            let selected =
                chosen.is_some_and(|chosen| chosen.character.account == friend.character.account);
            ui.push_id(("friend", friend.character.account.0), |ui| {
                if friend_row(ui, world, &content, words, friend, selected, now) {
                    done = Some(Done::Choose(friend.character.account));
                }
            });
        }
    });
    let chosen = chosen?;
    let name = &chosen.character.name;
    ui.add_space(GAP);
    ui.horizontal(|ui| {
        for action in tab::character_actions(world) {
            let label = action.label(words);
            if named(words, controls::button(ui, label), label, name).clicked() {
                done = Some(Done::Act(action, chosen.character.clone()));
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let remove = words.friends.remove_friend();
            if named(words, controls::danger(ui, remove), remove, name).clicked() {
                done = Some(Done::Remove(chosen.character.clone()));
            }
            let block = words.requests.block();
            if named(words, controls::button(ui, block), block, name).clicked() {
                done = Some(Done::Block(chosen.character.clone()));
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
    words: &messages::Messages,
    friend: &FriendRow,
    selected: bool,
    now: Option<UnixMillis>,
) -> bool {
    let online = friend.online_in;
    let state = match (online, now) {
        (Some(area), _) => words.friends.online_in(Mention::Area(area).span(content)),
        (None, Some(now)) => {
            let seen = words.time.since(now.since(friend.last_seen));
            LineText::plain(words.friends.offline_seen(seen))
        }
        (None, None) => LineText::plain(words.friends.offline()),
    };
    let look = controls::RowLook {
        selected,
        faint: online.is_none(),
        ..controls::RowLook::default()
    };
    let name = &friend.character.name;
    let shown = controls::row(ui, look, |ui| {
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let look = TextLook::role(TextRole::Small, ink::INK3);
                text::show(
                    ui,
                    world,
                    &state.styled(world.resource::<Content>()),
                    look,
                    None,
                );
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.add(
                        egui::Label::new(face::bold(name.as_str()))
                            .truncate()
                            .show_tooltip_when_elided(false),
                    );
                });
            });
        });
    });
    let response = shown
        .response
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| {
        let said = words.friends.row_named(name, state.plain_words(content));
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, said)
    });
    crate::systems::input::map::clicked(world, InputAction::Select, &response)
}

/// Asks the player whether to end the friendship with `player`, and ends it once they confirm.
fn remove(world: &mut World, player: SocietyMember) {
    let content = world.resource::<Content>().clone();
    let words = messages::of(&content);
    let name = &player.name;
    hud::confirm(
        world,
        Confirm {
            title: words.friends.remove_title(name).into(),
            body: vec![words.friends.remove_body(name).into()],
            confirm: words.interface.remove().to_owned(),
            cancel: words.friends.keep().to_owned(),
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
                ui.add(
                    egui::Label::new(name)
                        .truncate()
                        .show_tooltip_when_elided(false),
                );
            });
        });
    });
}

/// Names a button for what it does and to whom, so two alike in one list are told apart.
fn named(
    words: &messages::Messages,
    response: egui::Response,
    does: &str,
    name: &str,
) -> egui::Response {
    let enabled = response.enabled();
    let said = words.friends.button_for(does, name);
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, said.clone()));
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
