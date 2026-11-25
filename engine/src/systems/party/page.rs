//! The Party page of the Social tab: the invitations that wait while the player is in no party,
//! and their party with what its leader and its members may do, and who to invite.

use super::{invitations, invited, society};
use bevy::prelude::*;
use ui::egui::{self, Align, Layout, Rect, Sense, Ui, Vec2};
use ui::face::TextRole;
use ui::glyph::{self, Mark};
use ui::{controls, face, form, ink, input, sheet};

use super::frames::{self, Standing};
use super::{PartyMemberRow, PartyMessages, PartyRequest, PartyShown};
use crate::core::content::Content;
use crate::core::text::LineText;
use crate::core::text::{self, TextLook};
use crate::core::time::{self, UnixMillis};
use crate::systems::GameSettings;
use crate::systems::class;
use crate::systems::friend::search::{self, CharactersFound, FindCharacters};
use crate::systems::hud::{self, Confirm};
use crate::systems::input::map::{self, InputAction};
use crate::systems::item::inline;
use crate::systems::mention::Mention;
use crate::systems::messages;
use crate::systems::screen::ClientScreen;
use crate::systems::social_request::card::time_left;
use crate::systems::social_request::{AnswerSocialRequest, RequestAnswer, SocialRequestId};
use crate::systems::society::tab::{CharacterAction, SocialPage};
use crate::systems::society::{AccountNumber, RequestCard, SocietyMember, SocietyView};

pub struct PartyPage;

impl SocialPage for PartyPage {
    fn key(&self) -> &'static str {
        "party"
    }

    fn title<'a>(&self, messages: &'a messages::Messages) -> &'a str {
        messages.party.title()
    }

    fn on(&self, world: &World) -> bool {
        world.resource::<GameSettings>().party.is_some()
    }

    fn waiting(&self, world: &World) -> u32 {
        society(world).map_or(0, |view| invitations(&view).count() as u32)
    }

    fn hints(&self, world: &World) -> Vec<(InputAction, String)> {
        let messages = messages::of(world.resource::<Content>());
        let listed = society(world).is_some_and(|view| view.party.is_some());
        listed
            .then(|| (InputAction::Select, messages.party.hint_choose().to_owned()))
            .into_iter()
            .collect()
    }

    fn show(&self, ui: &mut Ui, world: &mut World) {
        let Some(view) = society(world) else {
            return;
        };
        hear(world);
        let content = world.resource::<Content>().clone();
        let words = messages::of(&content);
        let room = ui.available_rect_before_wrap();
        let ([party_at], invite_at) = sheet::columns(ui, room, [PARTY_WIDTH]);
        let mut done = None;
        let area = egui::ScrollArea::vertical().id_salt("party");
        sheet::scroll_pane(ui, party_at, area, |ui| {
            done = done.take().or(match &view.party {
                Some(party) => members(ui, world, words, &view, party),
                None => no_party(ui, world, words, &view),
            });
        });
        let area = egui::ScrollArea::vertical().id_salt("party invite");
        sheet::scroll_pane(ui, invite_at, area, |ui| {
            done = done.take().or(invite(ui, world, words, &view));
        });
        ui.allocate_rect(room, Sense::hover());
        if let Some(done) = done {
            done.apply(world);
        }
    }
}

/// Inviting a player a page lists to the player's party.
pub struct InviteToParty;

impl CharacterAction for InviteToParty {
    fn label<'a>(&self, messages: &'a messages::Messages) -> &'a str {
        messages.party.invite_to_party()
    }

    fn on(&self, world: &World) -> bool {
        world.resource::<GameSettings>().party.is_some()
    }

    fn act(&self, world: &mut World, player: &SocietyMember) {
        world.write_message(PartyRequest::Invite(player.account));
    }
}

pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<Inviting>()
        .add_systems(OnExit(ClientScreen::Area), forget);
}

/// A member's name as the party's lists show it: its leader's wears a crown.
pub(super) fn name(ui: &mut Ui, party: &PartyShown, member: &PartyMemberRow, name: egui::RichText) {
    ui.horizontal(|ui| {
        ui.add(
            egui::Label::new(name)
                .truncate()
                .show_tooltip_when_elided(false),
        );
        if party.leader == member.character.account {
            glyph::show(ui, Mark::Crown, CROWN, ink::GOLD);
        }
    });
}

/// A member's name as the party's lists read it out: its leader's says so.
pub(super) fn named(words: &PartyMessages, party: &PartyShown, member: &PartyMemberRow) -> String {
    if party.leader == member.character.account {
        words.leader_named(&member.character.name)
    } else {
        member.character.name.clone()
    }
}

const CROWN: f32 = 13.0;

/// Who the player looks for to invite, and the member of their party they chose.
#[derive(Resource, Default)]
struct Inviting {
    typed: String,
    /// When what is typed last changed, on the interface's clock.
    typed_at: f64,
    asked: Option<String>,
    found: Option<CharactersFound>,
    chosen: Option<AccountNumber>,
}

fn forget(mut inviting: ResMut<Inviting>) {
    *inviting = Inviting::default();
}

enum Done {
    Ask(PartyRequest),
    Answer(SocialRequestId, RequestAnswer),
    Choose(AccountNumber),
    Disband,
}

impl Done {
    fn apply(self, world: &mut World) {
        match self {
            Done::Ask(request) => {
                world.write_message(request);
            }
            Done::Answer(request, answer) => {
                world.write_message(AnswerSocialRequest { request, answer });
            }
            Done::Choose(account) => world.resource_mut::<Inviting>().chosen = Some(account),
            Done::Disband => disband(world),
        }
    }
}

/// Takes what the server found, and asks it for what is typed once the typing has paused.
fn hear(world: &mut World) {
    let found = world
        .resource_mut::<Messages<CharactersFound>>()
        .drain()
        .last();
    let now = world.resource::<Time<Real>>().elapsed_secs_f64();
    let content = world.resource::<Content>().clone();
    let mut inviting = world.resource_mut::<Inviting>();
    let searches = search::searches(&content, &inviting.typed);
    if let Some(found) = found {
        inviting.found = Some(found);
    }
    let paused = now - inviting.typed_at >= TYPING_PAUSE;
    let unasked = inviting.asked.as_deref() != Some(inviting.typed.as_str());
    if paused && unasked && searches {
        let typed = inviting.typed.clone();
        inviting.asked = Some(typed.clone());
        world.write_message(FindCharacters(typed));
    }
}

fn no_party(
    ui: &mut Ui,
    world: &World,
    words: &messages::Messages,
    view: &SocietyView,
) -> Option<Done> {
    let mut done = None;
    sheet::section(ui, words.party.your_party());
    ui.add(egui::Label::new(face::body(words.party.no_party())).wrap());
    ui.add_space(GAP);
    if controls::primary(ui, words.party.create()).clicked() {
        done = Some(Done::Ask(PartyRequest::Create));
    }
    let waiting: Vec<&RequestCard> = invitations(view).collect();
    if waiting.is_empty() {
        return done;
    }
    let now = time::server_now(world);
    ui.add_space(GAP);
    form::section_count(ui, words.party.invitations_heading(), waiting.len());
    for card in waiting {
        let name = &card.other.name;
        let left = now.map_or_else(String::new, |now| time_left(&words.time, card.until, now));
        ui.push_id(("invitation", card.id.0), |ui| {
            let said = words.party.invitation_line(&left, name);
            line(ui, &said, false, |ui| {
                let deny = words.requests.deny();
                if named_button(words, controls::button(ui, deny), deny, name).clicked() {
                    done = Some(Done::Answer(card.id, RequestAnswer::Deny));
                }
                let accept = words.requests.accept();
                if named_button(words, controls::primary(ui, accept), accept, name).clicked() {
                    done = Some(Done::Answer(card.id, RequestAnswer::Accept));
                }
            });
        });
    }
    done
}

fn members(
    ui: &mut Ui,
    world: &mut World,
    words: &messages::Messages,
    view: &SocietyView,
    party: &PartyShown,
) -> Option<Done> {
    let size = world
        .resource::<GameSettings>()
        .party
        .map_or(0, |rules| rules.size);
    let leads = party.leader == view.account;
    let heading = words.party.party_of_size(party.members.len(), size);
    sheet::section_with(ui, &heading, |ui| {
        if leads {
            controls::pill(ui, words.party.you_lead(), controls::Pill::Ink);
        }
    });
    let now = time::server_now(world);
    let chosen = world.resource::<Inviting>().chosen;
    let chosen = party.members.iter().find(|member| {
        Some(member.character.account) == chosen && member.character.account != view.account
    });
    let mut done = None;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for member in &party.members {
            let selected =
                chosen.is_some_and(|chosen| chosen.character.account == member.character.account);
            ui.push_id(("member", member.character.account.0), |ui| {
                if member_row(ui, world, words, party, member, selected, now) {
                    done = Some(Done::Choose(member.character.account));
                }
            });
        }
    });
    ui.add_space(GAP);
    if let Some(chosen) = chosen.filter(|_| leads) {
        let name = &chosen.character.name;
        ui.horizontal(|ui| {
            let make = words.party.make_leader();
            if named_button(words, controls::button(ui, make), make, name).clicked() {
                done = Some(Done::Ask(PartyRequest::Lead(chosen.character.account)));
            }
            let kick = words.party.remove_member();
            if named_button(words, controls::danger(ui, kick), kick, name).clicked() {
                done = Some(Done::Ask(PartyRequest::Remove(chosen.character.account)));
            }
        });
    } else if !leads {
        caption(ui, words.party.members_note());
    }
    ui.add_space(GAP * 2.0);
    ui.horizontal(|ui| {
        if controls::button(ui, words.party.leave()).clicked() {
            done = Some(Done::Ask(PartyRequest::Leave));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if leads && controls::danger(ui, words.party.disband()).clicked() {
                done = Some(Done::Disband);
            }
        });
    });
    done
}

/// A member's row. Whether the player chose it.
fn member_row(
    ui: &mut Ui,
    world: &mut World,
    words: &messages::Messages,
    party: &PartyShown,
    member: &PartyMemberRow,
    selected: bool,
    now: Option<UnixMillis>,
) -> bool {
    let standing = frames::standing(world, member);
    let seen = member
        .client
        .and_then(|client| frames::character_of(world, client));
    let content = world.resource::<Content>().clone();
    let class = seen.and_then(|seen| {
        let class = class::of(world, seen)?.get(&content).name.clone();
        Some(LineText::plain(
            words.party.class_level(&class, class::level(world, seen)),
        ))
    });
    let place = member.area.map(|area| Mention::Area(area).line(&content));
    let state = match &standing {
        Standing::Away(_) => LineText::plain(frames::said(words, &standing, now)),
        _ => joined([class, place].into_iter().flatten()),
    };
    let name = named(&words.party, party, member);
    let look = controls::RowLook {
        selected,
        faint: matches!(standing, Standing::Away(_)),
        ..controls::RowLook::default()
    };
    let shown = controls::row(ui, look, |ui| {
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Standing::Seen { health } = standing {
                    health_bar(ui, health, BAR_WIDTH);
                }
                ui.with_layout(Layout::top_down(Align::Min), |ui| {
                    self::name(
                        ui,
                        party,
                        member,
                        face::bold(member.character.name.as_str()),
                    );
                    let look = TextLook::role(TextRole::Small, ink::INK3);
                    inline::show_line(ui, world, &state, look, None);
                });
            });
        });
    });
    let response = shown
        .response
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| {
        let said = words.party.row_named(&name, state.plain_words(&content));
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, said)
    });
    map::clicked(world, InputAction::Select, &response)
}

/// Who to invite: anyone by name, and the friends who play right now.
fn invite(
    ui: &mut Ui,
    world: &mut World,
    words: &messages::Messages,
    view: &SocietyView,
) -> Option<Done> {
    sheet::section(ui, words.party.invite_heading());
    let now = world.resource::<Time<Real>>().elapsed_secs_f64();
    let server_now = time::server_now(world);
    let content = world.resource::<Content>().clone();
    let mut inviting = world.resource_mut::<Inviting>();
    let mut typed = inviting.typed.clone();
    let hint = words.friends.search_hint(search::least_letters(&content));
    input::search(ui, &mut typed, &hint, None);
    if typed != inviting.typed {
        inviting.typed = typed;
        inviting.typed_at = now;
    }
    let inviting = world.resource::<Inviting>();
    let mut done = None;
    if !inviting.typed.trim().is_empty() {
        ui.add_space(GAP);
        let found = inviting
            .found
            .as_ref()
            .filter(|found| found.starts == inviting.typed);
        match found {
            _ if !search::searches(&content, &inviting.typed) => caption(
                ui,
                &words.friends.type_more(search::least_letters(&content)),
            ),
            None => {
                caption(ui, words.friends.looking());
                ui.ctx().request_repaint();
            }
            Some(found) if found.characters.is_empty() => caption(ui, words.friends.nobody_found()),
            Some(found) => {
                for player in &found.characters {
                    ui.push_id(("found", player.account.0), |ui| {
                        done = done
                            .take()
                            .or(invite_row(ui, world, words, view, player, None, server_now));
                    });
                }
            }
        }
        return done;
    }
    let online: Vec<_> = view
        .friends
        .iter()
        .filter(|friend| friend.online_in.is_some())
        .collect();
    ui.add_space(GAP);
    form::section_count(ui, words.party.friends_online(), online.len());
    if online.is_empty() {
        caption(ui, words.party.none_online());
    }
    let content = world.resource::<Content>().clone();
    for friend in online {
        let note = if friend.in_a_party {
            Some(LineText::plain(words.party.in_another_party()))
        } else {
            friend
                .online_in
                .map(|area| Mention::Area(area).line(&content))
        };
        ui.push_id(("friend", friend.character.account.0), |ui| {
            let offered = (!friend.in_a_party).then_some(&friend.character);
            done = done.take().or(match offered {
                Some(player) => {
                    invite_row(ui, world, words, view, player, note.as_ref(), server_now)
                }
                None => {
                    line(ui, &friend.character.name, true, |ui| {
                        if let Some(note) = &note {
                            note_label(ui, world, note);
                        }
                    });
                    None
                }
            });
        });
    }
    done
}

/// A player to invite, with what stands between them and the party: nothing, an invitation that
/// waits, or that they are in it.
fn invite_row(
    ui: &mut Ui,
    world: &World,
    words: &messages::Messages,
    view: &SocietyView,
    player: &SocietyMember,
    note: Option<&LineText>,
    now: Option<UnixMillis>,
) -> Option<Done> {
    let name = &player.name;
    let member = view
        .party
        .iter()
        .flat_map(|party| &party.members)
        .any(|member| member.character.account == player.account);
    let waits = invited(view).find(|card| card.other.account == player.account);
    let mut done = None;
    line(ui, name, player.account == view.account, |ui| {
        if player.account == view.account {
            ui.label(face::small_faint(words.friends.you()));
        } else if member {
            controls::pill(ui, words.party.in_the_party(), controls::Pill::Verdigris);
        } else if let Some(card) = waits {
            let withdraw = words.friends.withdraw();
            if named_button(words, controls::button(ui, withdraw), withdraw, name).clicked() {
                done = Some(Done::Answer(card.id, RequestAnswer::TakeBack));
            }
            let left = now.map_or_else(String::new, |now| time_left(&words.time, card.until, now));
            let invited = words.party.invited(&left);
            controls::pill(ui, &invited, controls::Pill::Ink);
        } else {
            let invite = words.party.invite();
            if named_button(words, controls::primary(ui, invite), invite, name).clicked() {
                done = Some(Done::Ask(PartyRequest::Invite(player.account)));
            }
            if let Some(note) = note {
                note_label(ui, world, note);
            }
        }
    });
    done
}

fn note_label(ui: &mut Ui, world: &World, note: &LineText) {
    let look = TextLook::role(TextRole::Small, ink::INK3);
    text::show(
        ui,
        world,
        &note.styled(world.resource::<Content>()),
        look,
        None,
    );
}

/// Asks the leader whether to end the party, and ends it once they confirm.
fn disband(world: &mut World) {
    let content = world.resource::<Content>().clone();
    let words = messages::of(&content);
    hud::confirm(
        world,
        Confirm {
            title: words.party.disband_title().into(),
            body: vec![words.party.disband_body().into()],
            confirm: words.party.disband_confirm().to_owned(),
            cancel: words.friends.keep().to_owned(),
            answer: Box::new(|world, disbanded| {
                if disbanded {
                    world.write_message(PartyRequest::Disband);
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

fn joined(lines: impl IntoIterator<Item = LineText>) -> LineText {
    lines
        .into_iter()
        .reduce(|all, next| all.then(" · ").then(next))
        .unwrap_or_default()
}

/// Names a button for what it does and to whom, so two alike in one list are told apart.
fn named_button(
    words: &messages::Messages,
    response: egui::Response,
    does: &str,
    name: &str,
) -> egui::Response {
    let enabled = response.enabled();
    let said = words.party.button_for(does, name);
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, said.clone()));
    response
}

fn caption(ui: &mut Ui, text: &str) {
    form::caption(ui, text, ink::INK2);
}

const PARTY_WIDTH: f32 = 520.0;
/// A bar as full of health as `health` says, from none to all.
fn health_bar(ui: &mut Ui, health: f32, width: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, BAR_HEIGHT), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, BAR_HEIGHT / 2.0, ink::ink_wash(0.25));
    let full = Rect::from_min_size(rect.min, Vec2::new(rect.width() * health, rect.height()));
    painter.rect_filled(full, BAR_HEIGHT / 2.0, ink::WAX);
}

const BAR_WIDTH: f32 = 110.0;
const GAP: f32 = 8.0;
const LINE_HEIGHT: f32 = 28.0;
/// How long the typing rests before the server is asked, so it is never asked on every key.
const TYPING_PAUSE: f64 = 0.3;
const BAR_HEIGHT: f32 = 6.0;
