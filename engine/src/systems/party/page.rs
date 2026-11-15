//! The Party page of the Social tab: the invitations that wait while the player is in no party,
//! and their party with what its leader and its members may do, and who to invite.

use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ui::egui::{self, Align, Layout, Rect, Sense, Ui, Vec2};
use ui::glyph::{self, Mark};
use ui::{controls, face, form, ink, input, sheet};

use super::frames::{self, Standing};
use super::{PartyMemberRow, PartyRequest, PartyShown};
use crate::core::content::Content;
use crate::core::time::{self, UnixMillis};
use crate::systems::GameSettings;
use crate::systems::class;
use crate::systems::friend::search::{self, FindPlayers, PlayersFound};
use crate::systems::hud::{self, Confirm};
use crate::systems::input::map::{self, InputAction};
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;
use crate::systems::social_request::card::time_left;
use crate::systems::social_request::{
    AnswerSocialRequest, RequestAnswer, SocialRequestId, SocialRequestKind,
};
use crate::systems::society::tab::{PlayerAction, SocialPage};
use crate::systems::society::{AccountNumber, RequestCard, SocietyMember, SocietyView};

pub struct PartyPage;

impl SocialPage for PartyPage {
    fn title(&self) -> &'static str {
        "Party"
    }

    fn on(&self, world: &World) -> bool {
        world.resource::<GameSettings>().party.is_some()
    }

    fn waiting(&self, world: &World) -> u32 {
        view(world).map_or(0, |view| invitations(&view).count() as u32)
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
        let ([party_at], invite_at) = sheet::columns(ui, room, [PARTY_WIDTH]);
        let mut done = None;
        let area = egui::ScrollArea::vertical().id_salt("party");
        sheet::scroll_pane(ui, party_at, area, |ui| {
            done = done.take().or(match &view.party {
                Some(party) => members(ui, world, &view, party),
                None => no_party(ui, world, &view),
            });
        });
        let area = egui::ScrollArea::vertical().id_salt("party invite");
        sheet::scroll_pane(ui, invite_at, area, |ui| {
            done = done.take().or(invite(ui, world, &view));
        });
        ui.allocate_rect(room, Sense::hover());
        if let Some(done) = done {
            done.apply(world);
        }
    }
}

/// Inviting a player a page lists to the player's party.
pub struct InviteToParty;

impl PlayerAction for InviteToParty {
    fn label(&self) -> &'static str {
        "Invite to party"
    }

    fn on(&self, world: &World) -> bool {
        world.resource::<GameSettings>().party.is_some()
    }

    fn act(&self, world: &mut World, player: &SocietyMember) {
        world.write_message(PartyRequest::Invite(player.account));
    }
}

/// What the player sees of parties, for a driver of the client.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
pub struct ShownParty {
    /// The members of the party the player is in, in the order they joined, the player among
    /// them.
    pub members: Vec<ShownPartyMember>,
    /// Who invites the player to a party.
    pub invitations: Vec<String>,
    /// Who the player invited and waits for.
    pub invited: Vec<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ShownPartyMember {
    pub name: String,
    pub leads: bool,
    /// The area they play in, while their player is at their character.
    pub area: Option<String>,
}

pub fn shown(world: &World) -> ShownParty {
    let Some(view) = view(world) else {
        return ShownParty::default();
    };
    let content = world.resource::<Content>();
    let names =
        |cards: Vec<&RequestCard>| cards.iter().map(|card| card.other.name.clone()).collect();
    ShownParty {
        members: view
            .party
            .iter()
            .flat_map(|party| party.members.iter().map(move |member| (party, member)))
            .map(|(party, member)| ShownPartyMember {
                name: member.player.name.clone(),
                leads: party.leader == member.player.account,
                area: member.area.map(|area| area.get(content).name.clone()),
            })
            .collect(),
        invitations: names(invitations(&view).collect()),
        invited: names(invited(&view).collect()),
    }
}

pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<Inviting>()
        .add_systems(OnExit(ClientScreen::Area), forget);
}

/// A member's name as the party's lists show it: its leader's wears a crown.
pub(super) fn name(ui: &mut Ui, party: &PartyShown, member: &PartyMemberRow, name: egui::RichText) {
    ui.horizontal(|ui| {
        ui.add(egui::Label::new(name).truncate());
        if party.leader == member.player.account {
            glyph::show(ui, Mark::Crown, CROWN, ink::GOLD);
        }
    });
}

/// A member's name as the party's lists read it out: its leader's says so.
pub(super) fn named(party: &PartyShown, member: &PartyMemberRow) -> String {
    if party.leader == member.player.account {
        format!("{}, {}", member.player.name, Mark::Crown.word())
    } else {
        member.player.name.clone()
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
    found: Option<PlayersFound>,
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

fn view(world: &World) -> Option<SocietyView> {
    session::my_character(world)?.get::<SocietyView>().cloned()
}

fn invitation(view: &SocietyView) -> impl Iterator<Item = &RequestCard> {
    view.requests
        .iter()
        .filter(|card| card.kind == SocialRequestKind::PartyInvitation)
}

/// The invitations the player is asked.
fn invitations(view: &SocietyView) -> impl Iterator<Item = &RequestCard> {
    invitation(view).filter(|card| !card.mine)
}

/// The invitations the player sent.
fn invited(view: &SocietyView) -> impl Iterator<Item = &RequestCard> {
    invitation(view).filter(|card| card.mine)
}

/// Takes what the server found, and asks it for what is typed once the typing has paused.
fn hear(world: &mut World) {
    let found = world
        .resource_mut::<Messages<PlayersFound>>()
        .drain()
        .last();
    let now = world.resource::<Time<Real>>().elapsed_secs_f64();
    let mut inviting = world.resource_mut::<Inviting>();
    if let Some(found) = found {
        inviting.found = Some(found);
    }
    let paused = now - inviting.typed_at >= TYPING_PAUSE;
    let unasked = inviting.asked.as_deref() != Some(inviting.typed.as_str());
    if paused && unasked && search::searches(&inviting.typed) {
        let typed = inviting.typed.clone();
        inviting.asked = Some(typed.clone());
        world.write_message(FindPlayers(typed));
    }
}

fn no_party(ui: &mut Ui, world: &World, view: &SocietyView) -> Option<Done> {
    let mut done = None;
    sheet::section(ui, "Your party");
    ui.add(
        egui::Label::new(face::body(
            "You are in no party. A party shares drops, experience and the kills its quests count.",
        ))
        .wrap(),
    );
    ui.add_space(GAP);
    if controls::primary(ui, "Create a party").clicked() {
        done = Some(Done::Ask(PartyRequest::Create));
    }
    let waiting: Vec<&RequestCard> = invitations(view).collect();
    if waiting.is_empty() {
        return done;
    }
    let now = time::server_now(world);
    ui.add_space(GAP);
    form::section_count(ui, "Invitations", waiting.len());
    for card in waiting {
        let name = &card.other.name;
        let left = now.map_or_else(String::new, |now| time_left(card.until, now));
        ui.push_id(("invitation", card.id.0), |ui| {
            line(ui, &format!("{name} · {left}"), false, |ui| {
                if named_button(controls::button(ui, "Deny"), "Deny", name).clicked() {
                    done = Some(Done::Answer(card.id, RequestAnswer::Deny));
                }
                if named_button(controls::primary(ui, "Accept"), "Accept", name).clicked() {
                    done = Some(Done::Answer(card.id, RequestAnswer::Accept));
                }
            });
        });
    }
    done
}

fn members(ui: &mut Ui, world: &mut World, view: &SocietyView, party: &PartyShown) -> Option<Done> {
    let size = world
        .resource::<GameSettings>()
        .party
        .map_or(0, |rules| rules.size);
    let leads = party.leader == view.account;
    let heading = format!("Your party · {} of {size}", party.members.len());
    sheet::section_with(ui, &heading, |ui| {
        if leads {
            controls::pill(ui, "You lead", controls::Pill::Ink);
        }
    });
    let now = time::server_now(world);
    let chosen = world.resource::<Inviting>().chosen;
    let chosen = party.members.iter().find(|member| {
        Some(member.player.account) == chosen && member.player.account != view.account
    });
    let mut done = None;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for member in &party.members {
            let selected =
                chosen.is_some_and(|chosen| chosen.player.account == member.player.account);
            ui.push_id(("member", member.player.account.0), |ui| {
                if member_row(ui, world, party, member, selected, now) {
                    done = Some(Done::Choose(member.player.account));
                }
            });
        }
    });
    ui.add_space(GAP);
    if let Some(chosen) = chosen.filter(|_| leads) {
        let name = &chosen.player.name;
        ui.horizontal(|ui| {
            if named_button(controls::button(ui, "Make leader"), "Make leader", name).clicked() {
                done = Some(Done::Ask(PartyRequest::Lead(chosen.player.account)));
            }
            let remove = controls::danger(ui, "Remove from party");
            if named_button(remove, "Remove from party", name).clicked() {
                done = Some(Done::Ask(PartyRequest::Remove(chosen.player.account)));
            }
        });
    } else if !leads {
        caption(
            ui,
            "Any member may invite. Only the leader removes, hands over, and disbands.",
        );
    }
    ui.add_space(GAP * 2.0);
    ui.horizontal(|ui| {
        if controls::button(ui, "Leave party").clicked() {
            done = Some(Done::Ask(PartyRequest::Leave));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if leads && controls::danger(ui, "Disband party").clicked() {
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
        Some(format!("{class} {}", class::level(world, seen)))
    });
    let place = member.area.map(|area| area.get(&content).name.clone());
    let state = match &standing {
        Standing::Away(_) => frames::said(&standing, now),
        _ => [class, place]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · "),
    };
    let name = named(party, member);
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
                    self::name(ui, party, member, face::bold(member.player.name.as_str()));
                    ui.add(
                        egui::Label::new(face::small(state.as_str()).color(ink::INK3)).truncate(),
                    );
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
    map::clicked(world, InputAction::Select, &response)
}

/// Who to invite: anyone by name, and the friends who play right now.
fn invite(ui: &mut Ui, world: &mut World, view: &SocietyView) -> Option<Done> {
    sheet::section(ui, "Invite");
    let now = world.resource::<Time<Real>>().elapsed_secs_f64();
    let server_now = time::server_now(world);
    let mut inviting = world.resource_mut::<Inviting>();
    let mut typed = inviting.typed.clone();
    input::search(ui, &mut typed, "A name, two characters or more", None);
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
            _ if !search::searches(&inviting.typed) => caption(ui, "Two characters or more"),
            None => {
                caption(ui, "Looking");
                ui.ctx().request_repaint();
            }
            Some(found) if found.players.is_empty() => caption(ui, "Nobody by that name"),
            Some(found) => {
                for player in &found.players {
                    ui.push_id(("found", player.account.0), |ui| {
                        done = done
                            .take()
                            .or(invite_row(ui, view, player, None, server_now));
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
    form::section_count(ui, "Friends online", online.len());
    if online.is_empty() {
        caption(ui, "None of your friends play right now.");
    }
    let content = world.resource::<Content>().clone();
    for friend in online {
        let area = friend.online_in.map(|area| area.get(&content).name.clone());
        let note = if friend.in_a_party {
            Some("in another party".to_owned())
        } else {
            area
        };
        ui.push_id(("friend", friend.player.account.0), |ui| {
            let offered = (!friend.in_a_party).then_some(&friend.player);
            done = done.take().or(match offered {
                Some(player) => invite_row(ui, view, player, note.as_deref(), server_now),
                None => {
                    line(ui, &friend.player.name, true, |ui| {
                        ui.label(face::small_faint(note.as_deref().unwrap_or_default()));
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
    view: &SocietyView,
    player: &SocietyMember,
    note: Option<&str>,
    now: Option<UnixMillis>,
) -> Option<Done> {
    let name = &player.name;
    let member = view
        .party
        .iter()
        .flat_map(|party| &party.members)
        .any(|member| member.player.account == player.account);
    let waits = invited(view).find(|card| card.other.account == player.account);
    let mut done = None;
    line(ui, name, player.account == view.account, |ui| {
        if player.account == view.account {
            ui.label(face::small_faint("You"));
        } else if member {
            controls::pill(ui, "In the party", controls::Pill::Verdigris);
        } else if let Some(card) = waits {
            if named_button(controls::button(ui, "Withdraw"), "Withdraw", name).clicked() {
                done = Some(Done::Answer(card.id, RequestAnswer::TakeBack));
            }
            let left = now.map_or_else(String::new, |now| time_left(card.until, now));
            controls::pill(ui, &format!("Invited · {left}"), controls::Pill::Ink);
        } else {
            if named_button(controls::primary(ui, "Invite"), "Invite", name).clicked() {
                done = Some(Done::Ask(PartyRequest::Invite(player.account)));
            }
            if let Some(note) = note {
                ui.label(face::small_faint(note));
            }
        }
    });
    done
}

/// Asks the leader whether to end the party, and ends it once they confirm.
fn disband(world: &mut World) {
    hud::confirm(
        world,
        Confirm {
            title: "Disband the party?".to_owned(),
            body: vec!["It ends for everyone in it.".to_owned()],
            confirm: "Disband".to_owned(),
            cancel: "Keep".to_owned(),
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
                ui.add(egui::Label::new(name).truncate());
            });
        });
    });
}

/// Names a button for what it does and to whom, so two alike in one list are told apart.
fn named_button(response: egui::Response, does: &str, name: &str) -> egui::Response {
    let enabled = response.enabled();
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, format!("{does} {name}"))
    });
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
