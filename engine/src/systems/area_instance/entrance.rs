//! The entrance panel: where a player starts an instance of an instanced area. It opens before every
//! start, as a panel of the conversation the entrance's script holds, with a tray for the items
//! starting takes. What the entrance says follows what is inserted, and starting opens once it is
//! what the area takes: alone, or, for a party's leader, with the party they gather. A player who
//! started an instance too lately to start another sees how long they wait, and only a way out.

use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ui::egui::{self, Align, Layout, Pos2, Rect, Ui, UiBuilder, Vec2};
use ui::face::TextRole;
use ui::{controls, ink, sheet};

use super::roster::{InstanceAsk, InstanceDoor};
use super::{AreaInstanceEntry, InstanceCooldowns, PlayerCount};
use bevy_egui::EguiPrimaryContextPass;

use crate::core::content::Content;
use crate::core::gui::{self, Band};
use crate::core::script::DialogueCamera;
use crate::core::script::{Answer, BoxEnding, REFUSED};
use crate::core::script::{FlowKey, Instances};
use crate::core::text::LineText;
use crate::core::text::{self, TextLook};
use crate::core::time::{self, UnixMillis, WallClock};
use crate::core::units::{MeterPos, Seconds};
use crate::systems::area::AreaId;
use crate::systems::dialogue;
use crate::systems::dialogue::stage;
use crate::systems::dialogue::windows::{self, Opens};
use crate::systems::hud::WindowKey;
use crate::systems::item::{Inventory, ItemStack};
use crate::systems::mention::Mention;
use crate::systems::messages::{self, Line, Messages, Rich};
use crate::systems::player::{self, CharacterAccount, PlayedBy, RequestNeeds, session};
use crate::systems::prompt::{self, Asked, PromptKind, tray};
use crate::systems::reach::{self, Tether};
use crate::systems::screen::ClientScreen;
use crate::systems::settings::Placed;
use crate::systems::society::{self, ServerSociety, SocietyAsk};
use crate::systems::window::{OpenWindow, WindowStep, WindowSteps};
use crate::systems::{GameSettings, movement, stat};

pub(crate) fn register(app: &mut App) {
    use bevy_replication::{AppMessageExt, AppReplicationExt};
    app.replicate_private::<EntranceView>()
        .add_client_message::<EntranceChoice>();
}

pub(crate) fn plugin(app: &mut App) {
    app.add_systems(
        EguiPrimaryContextPass,
        draw.in_set(Band::Hud).run_if(in_state(ClientScreen::Area)),
    );
}

/// What the entrance panel shows its player.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct EntranceView {
    pub step: WindowStep,
    pub area: AreaId,
    pub players: PlayerCount,
    pub lasts: Option<Seconds>,
    pub party: EntranceParty,
    /// When the player may start an instance of the area again, while they wait to.
    pub ready_at: Option<UnixMillis>,
}

impl EntranceView {
    /// Whether the area may be done alone.
    pub fn alone_opens(&self) -> bool {
        self.players.least <= 1
    }
}

/// How the player at the entrance stands with a party.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum EntranceParty {
    /// In none.
    Alone,
    /// They lead a party of `size`, of whom those named stand within their sight.
    Leads { size: u32, nearby: Vec<String> },
    /// They are in a party `leader` leads.
    Member { leader: String },
    /// Another member holds a place in an open instance of this area.
    Inside,
}

/// The player's choice at the entrance panel, with what they inserted.
#[derive(JsonSchema, Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct EntranceChoice {
    pub step: WindowStep,
    pub answer: EntranceAnswer,
    pub offer: Vec<ItemStack>,
}

#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntranceAnswer {
    GoAlone,
    Gather,
}

/// Opens the entrance panel for `character` as an entrance's script asks, the script waiting on
/// what they choose. `owner` is what runs the script, the entrance itself where it is a thing in
/// the world; `camera` how the conversation holding the panel is seen.
pub(crate) fn enter(
    world: &mut World,
    character: Entity,
    owner: Option<Entity>,
    entry: AreaInstanceEntry,
    camera: DialogueCamera,
) {
    let Some(account) = world
        .get::<CharacterAccount>(character)
        .map(|a| a.0.clone())
    else {
        refuse(world, entry.flow);
        return;
    };
    let content = world.resource::<Content>().clone();
    let area = entry.to.area;
    let Some(rules) = area.get(&content).instance.clone() else {
        refuse(world, entry.flow);
        return;
    };
    let society = world.resource::<ServerSociety>().clone();
    let society = society.read();
    let parties = society.parties();
    let party = match parties.of_account(&account) {
        None => EntranceParty::Alone,
        Some(_) if society.instances().party_inside(parties, &account, area) => {
            EntranceParty::Inside
        }
        Some(party) if parties.leads(&account) => EntranceParty::Leads {
            size: party.members.len() as u32,
            nearby: nearby(world, character, party.accounts()),
        },
        Some(party) => EntranceParty::Member {
            leader: society
                .accounts()
                .get(&party.leader)
                .map(|leader| leader.name.clone())
                .unwrap_or_default(),
        },
    };
    drop(society);
    if world.get::<EntranceSession>(character).is_some() {
        close(world, character);
    }
    let asked = Asked {
        title: area.get(&content).name.clone(),
        text: None,
        slots: rules.entry.needs.len() as u32,
        camera,
        on_leave: REFUSED,
    };
    if !prompt::open(world, character, entry.flow, asked, PromptKind::Hosted) {
        refuse(world, entry.flow);
        return;
    }
    let step = world.resource_mut::<WindowSteps>().deal();
    let now = world.resource::<WallClock>().now;
    let ready_at = world
        .get::<InstanceCooldowns>(character)
        .and_then(|cooldowns| cooldowns.left(area, now))
        .map(|left| now.after(left));
    let tether = Tether::opened_by(world, character, owner);
    let view = EntranceView {
        step,
        area,
        players: rules.players,
        lasts: rules.lasts,
        party,
        ready_at,
    };
    world.entity_mut(character).insert((
        EntranceSession {
            step,
            entry,
            tether,
        },
        view,
    ));
}

/// What players of this world chose at their entrance panels: a choice whose items are what the
/// area takes, made at the entrance by a player who need not wait, closes the panel and goes on
/// alone or to gather the party. Any other is refused, the panel staying open.
pub fn requests(world: &mut World) {
    player::player_requests::<EntranceChoice>(
        world,
        RequestNeeds::Nothing,
        |world, player, choice| {
            let Some(session) = world.get::<EntranceSession>(player) else {
                return Ok(());
            };
            if session.step != choice.step {
                return Ok(());
            }
            if let Err(why) = admits(world, player, session, &choice.offer) {
                prompt::ask_again(world, player);
                return Err(why);
            }
            let session = world
                .entity_mut(player)
                .take::<EntranceSession>()
                .expect("the session just read");
            let flow = session.entry.flow;
            world.entity_mut(player).remove::<EntranceView>();
            prompt::close(world, player);
            dialogue::close(world, flow, player, BoxEnding::Ended);
            let ask = match choice.answer {
                EntranceAnswer::GoAlone => InstanceAsk::GoAlone(session.entry),
                EntranceAnswer::Gather => InstanceAsk::Gather {
                    entry: session.entry,
                    door: session.tether.and_then(|tether| door(world, tether)),
                },
            };
            match world.get::<PlayedBy>(player).map(|by| by.client) {
                Some(client) => society::ask(world, client, SocietyAsk::Instance(ask)),
                None => refuse(world, flow),
            }
            Ok(())
        },
    );
}

/// The entrance panel stays open while its player is in the conversation that holds it and can
/// answer it where they stand: standing, and within the entrance's reach. Closed by a conversation
/// that ended, it takes the script's refusal with it.
impl OpenWindow for EntranceSession {
    fn kept(&self, world: &World, player: Entity) -> bool {
        self.in_conversation(world, player)
            && reach::lapse(world, player, self.tether.as_ref()).is_none()
    }

    fn close(world: &mut World, player: Entity) {
        let in_conversation = world
            .get::<EntranceSession>(player)
            .is_some_and(|session| session.in_conversation(world, player));
        if in_conversation {
            close(world, player);
        } else {
            world
                .entity_mut(player)
                .remove::<(EntranceSession, EntranceView)>();
        }
    }
}

/// Closes `player`'s entrance panel, if one is open, its script taking the refusal: for one who
/// leaves the world it was opened in, or leaves their character.
pub fn close(world: &mut World, player: Entity) {
    let Ok(mut held) = world.get_entity_mut(player) else {
        return;
    };
    let Some(session) = held.take::<EntranceSession>() else {
        return;
    };
    held.remove::<EntranceView>();
    let flow = session.entry.flow;
    prompt::close(world, player);
    dialogue::close(world, flow, player, BoxEnding::Ended);
    refuse(world, flow);
}

/// The entrance of an instance the player stands before, and how the party stands to it.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ShownInstanceEntrance {
    pub area: String,
    pub least: u32,
    pub most: u32,
    /// How long the player waits before they may start the area again.
    pub waits: Option<Seconds>,
    /// Whether Gather the party is offered: to a party's leader alone.
    pub gather: bool,
    /// How many members of the party stand within the player's sight.
    pub nearby: u32,
    /// Whether another member of the party is inside an instance of the area.
    pub party_inside: bool,
}

pub fn shown(world: &World) -> Option<ShownInstanceEntrance> {
    let view = session::my_character(world)?.get::<EntranceView>()?;
    let content = world.resource::<Content>();
    Some(ShownInstanceEntrance {
        area: view.area.get(content).name.clone(),
        least: view.players.least,
        most: view.players.most,
        waits: waits(world, view),
        gather: matches!(view.party, EntranceParty::Leads { .. }),
        nearby: match &view.party {
            EntranceParty::Leads { nearby, .. } => nearby.len() as u32,
            _ => 0,
        },
        party_inside: view.party == EntranceParty::Inside,
    })
}

/// The entrance panel as it shows what the player has inserted, for a driver of the client.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ShownEntrancePanel {
    /// What the entrance says of what is inserted, or of how long the player waits.
    pub says: String,
    /// Whether what is inserted is what the area takes, so starting is open.
    pub takes: bool,
    /// Whether Go alone is open.
    pub alone: bool,
}

pub fn shown_panel(world: &World) -> Option<ShownEntrancePanel> {
    let view = session::my_character(world)?.get::<EntranceView>()?;
    let content = world.resource::<Content>();
    let state = state(world, view);
    Some(ShownEntrancePanel {
        says: says(content, view.area, &state).plain_words(content),
        takes: state == Inserted::Right,
        alone: alone_open(world, view, &state),
    })
}

/// The panel's server side: the script waiting on the choice, and where the entrance stands.
#[derive(Component)]
pub struct EntranceSession {
    step: WindowStep,
    entry: AreaInstanceEntry,
    tether: Option<Tether>,
}

impl EntranceSession {
    fn in_conversation(&self, world: &World, player: Entity) -> bool {
        dialogue::flow_of(world, player) == Some(self.entry.flow)
    }
}

/// Whether `player` may start the session's area with `offer`, or why not, as the game words it.
fn admits(
    world: &World,
    player: Entity,
    session: &EntranceSession,
    offer: &[ItemStack],
) -> Result<(), LineText> {
    let content = world.resource::<Content>();
    let messages = &messages::of(content).instance;
    let area = session.entry.to.area;
    let Some(rules) = &area.get(content).instance else {
        return Err(messages.busy().into());
    };
    let now = world.resource::<WallClock>().now;
    if let Some(left) = world
        .get::<InstanceCooldowns>(player)
        .and_then(|cooldowns| cooldowns.left(area, now))
    {
        return Err(messages
            .cooldown(messages::of(content).time.exact(left))
            .into());
    }
    if reach::lapse(world, player, session.tether.as_ref()).is_some() {
        return Err(messages.busy().into());
    }
    let named = Mention::Area(area).span(content);
    if !rules.entry.takes(offer) {
        return Err(messages.wrong_items(named));
    }
    let covered = Inventory::of(world, player)
        .exchange(content, offer, &[])
        .is_ok();
    if !covered {
        return Err(messages.items_gone(named));
    }
    Ok(())
}

fn refuse(world: &mut World, flow: FlowKey) {
    world
        .resource_mut::<Instances>()
        .answer(flow, Answer::Output(REFUSED));
}

/// The names of the members of `party` who stand on their feet within sight of `character`.
fn nearby<'a>(
    world: &mut World,
    character: Entity,
    party: impl Iterator<Item = &'a crate::systems::account::AccountId>,
) -> Vec<String> {
    let sight = world.resource::<GameSettings>().sight;
    let Some(here) = movement::position(world, character) else {
        return Vec::new();
    };
    let party: Vec<_> = party.cloned().collect();
    let mut members = world.query::<(Entity, &CharacterAccount, &crate::systems::actor::Name)>();
    members
        .iter(world)
        .filter(|(member, account, _)| *member != character && party.contains(&account.0))
        .filter(|(member, ..)| {
            movement::position(world, *member).is_some_and(|at| at.distance(here) <= sight)
                && !stat::is_dead(world, *member)
        })
        .map(|(_, _, name)| name.name.clone())
        .collect()
}

fn door(world: &World, tether: Tether) -> Option<InstanceDoor> {
    Some(InstanceDoor {
        at: movement::position(world, tether.to)?,
        reach: tether.range,
    })
}

/// What the player has inserted at the entrance, as far as what it says follows it.
#[derive(Clone, Debug, PartialEq)]
enum Inserted {
    /// They wait out the area's cooldown, `left` of it.
    Waiting {
        left: Seconds,
    },
    Nothing,
    Wrong(Vec<ItemStack>),
    Right,
}

fn view(world: &World) -> Option<EntranceView> {
    prompt::view(world).filter(|prompt| prompt.kind == PromptKind::Hosted)?;
    session::my_character(world)?.get::<EntranceView>().cloned()
}

fn waits(world: &World, view: &EntranceView) -> Option<Seconds> {
    let now = time::server_now(world)?;
    view.ready_at
        .filter(|ready| *ready > now)
        .map(|ready| ready.since(now))
}

/// Whether Go alone may be chosen now: the right items are in, the area lets one player in, and no
/// answer is on its way.
fn alone_open(world: &World, view: &EntranceView, state: &Inserted) -> bool {
    *state == Inserted::Right && view.alone_opens() && !tray::waiting(world)
}

fn state(world: &World, view: &EntranceView) -> Inserted {
    if let Some(left) = waits(world, view) {
        return Inserted::Waiting { left };
    }
    let content = world.resource::<Content>();
    let offer = tray::offer(world);
    let takes = view
        .area
        .get(content)
        .instance
        .as_ref()
        .is_some_and(|rules| rules.entry.takes(&offer));
    match (offer.is_empty(), takes) {
        (true, _) => Inserted::Nothing,
        (false, true) => Inserted::Right,
        (false, false) => Inserted::Wrong(offer),
    }
}

/// What the entrance of `area` says of what is inserted: the area's own words for it, or how long
/// the player waits.
fn says(content: &Content, area: AreaId, state: &Inserted) -> LineText {
    let messages = messages::of(content);
    let Some(rules) = &area.get(content).instance else {
        return LineText::default();
    };
    let entry = &rules.entry;
    match state {
        Inserted::Waiting { left } => messages
            .instance
            .cooldown(messages.time.exact(*left))
            .into(),
        Inserted::Nothing => entry.empty.text().into(),
        Inserted::Wrong(offer) => {
            let items = offer.iter().map(|stack| stack.named()).collect();
            let listed = Line::from(messages.interface.listed(items));
            entry.wrong.line(vec![("items", Rich::from(listed))])
        }
        Inserted::Right => entry.right.text().into(),
    }
}

fn draw(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let content = world.resource::<Content>().clone();
    let opens = Opens {
        key: WindowKey::Entrance,
        of: Placed::Entrance,
        size: SHEET,
        min: SHEET_LEAST,
    };
    let open = view(world);
    let title = |shown: &EntranceView| shown.area.get(&content).name.clone();
    windows::window(world, &gui, opens, open, title, |ui, world, _| {
        show(ui, world)
    });
}

/// The panel: how many it opens for and how long it lasts, what the entrance says, the tray, and
/// the ways in beside the way out, with the inventory to insert from. A player who waits out the
/// cooldown sees how long, and only the way out.
fn show(ui: &mut Ui, world: &mut World) {
    let Some(view) = view(world) else {
        return;
    };
    let content = world.resource::<Content>().clone();
    let messages = messages::of(&content);
    let state = state(world, &view);
    let waiting = matches!(state, Inserted::Waiting { .. });
    egui::Frame::new()
        .inner_margin(sheet::PAGE_MARGIN)
        .show(ui, |ui| {
            let room = ui.available_rect_before_wrap();
            let inventory_at = Rect::from_min_max(
                Pos2::new(room.max.x - tray::INVENTORY_WIDTH, room.min.y),
                room.max,
            );
            let entrance_at = if waiting {
                room
            } else {
                Rect::from_min_max(
                    room.min,
                    Pos2::new(inventory_at.min.x - sheet::GUTTER, room.max.y),
                )
            };
            let column = |rect| {
                UiBuilder::new()
                    .max_rect(rect)
                    .layout(Layout::top_down(Align::Min))
            };
            ui.scope_builder(column(entrance_at), |ui| {
                chips(ui, messages, &view);
                ui.add_space(8.0);
                let look = TextLook::role(TextRole::Body, ink::INK);
                let said = says(&content, view.area, &state).styled(&content);
                text::show(ui, world, &said, look, None);
                ui.add_space(8.0);
                if !waiting {
                    let slots = view
                        .area
                        .get(&content)
                        .instance
                        .as_ref()
                        .map_or(1, |rules| rules.entry.needs.len() as u32);
                    tray::slots(ui, world, slots);
                    ui.add_space(8.0);
                    party_lines(ui, world, messages, &view);
                }
                ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                    foot(ui, world, messages, &view, &state);
                });
            });
            if !waiting {
                ui.scope_builder(column(inventory_at), |ui| tray::inventory(ui, world, false));
            }
            ui.advance_cursor_after_rect(room);
        });
}

/// How many players the area opens for, and how long an instance lasts where it is timed.
fn chips(ui: &mut Ui, messages: &Messages, view: &EntranceView) {
    let instance = &messages.instance;
    let players = match (view.players.least, view.players.most) {
        (least, most) if least == most => instance.players_exact(least),
        (least, most) => instance.players_range(least, most),
    };
    ui.horizontal(|ui| {
        controls::pill(ui, &players, controls::Pill::Ink);
        if let Some(lasts) = view.lasts {
            let limit = instance.time_limit(messages.time.rough(lasts));
            controls::pill(ui, &limit, controls::Pill::Ink);
        }
    });
}

/// What the entrance tells the player of their party: who of it stands near a leader, who leads
/// a member's party, or that another member is inside an instance already.
fn party_lines(ui: &mut Ui, world: &World, messages: &Messages, view: &EntranceView) {
    let content = world.resource::<Content>();
    let area = Mention::Area(view.area).span(content);
    let instance = &messages.instance;
    let lines: Vec<LineText> = match &view.party {
        EntranceParty::Alone => (!view.alone_opens())
            .then(|| LineText::plain(instance.form_party(view.players.least)))
            .into_iter()
            .collect(),
        EntranceParty::Leads { size, nearby } => {
            let mut lines = vec![LineText::plain(instance.lead_party(*size))];
            if !nearby.is_empty() {
                let names = messages.interface.listed_words(nearby.clone());
                lines.push(LineText::plain(instance.stand_near(nearby.len(), names)));
            }
            lines
        }
        EntranceParty::Member { leader } => {
            vec![LineText::plain(instance.leader_only(leader))]
        }
        EntranceParty::Inside => vec![instance.already_inside(area)],
    };
    let look = TextLook::role(TextRole::Small, ink::INK2);
    for line in lines {
        text::show(ui, world, &line.styled(content), look, None);
    }
}

/// The ways in, each beside what it means, over the way out. A way in opens once what is
/// inserted is what the area takes; going alone also once the area may be done alone.
fn foot(
    ui: &mut Ui,
    world: &mut World,
    messages: &Messages,
    view: &EntranceView,
    state: &Inserted,
) {
    let content = world.resource::<Content>().clone();
    let instance = &messages.instance;
    let takes = *state == Inserted::Right;
    let mut chosen = None;
    ui.horizontal(|ui| {
        if controls::button(ui, messages.interface.cancel()).clicked() {
            stage::leave(world);
        }
        if matches!(state, Inserted::Waiting { .. }) {
            return;
        }
        let alone = match view.party {
            EntranceParty::Inside => instance.alone_own_instance(),
            _ => instance.go_alone(),
        };
        let opens = alone_open(world, view, state);
        if ui
            .add_enabled_ui(opens, |ui| controls::primary(ui, alone))
            .inner
            .clicked()
        {
            chosen = Some(EntranceAnswer::GoAlone);
        }
        if let EntranceParty::Leads { nearby, .. } = &view.party {
            let gather = instance.gather(nearby.len());
            let opens = takes && !tray::waiting(world);
            if ui
                .add_enabled_ui(opens, |ui| controls::primary(ui, &gather))
                .inner
                .clicked()
            {
                chosen = Some(EntranceAnswer::Gather);
            }
        }
    });
    if !matches!(state, Inserted::Waiting { .. }) {
        let notes: Vec<LineText> = match &view.party {
            EntranceParty::Leads { .. } => vec![
                LineText::plain(instance.alone_note()),
                LineText::plain(instance.gather_note()),
            ],
            _ if !view.alone_opens() => vec![
                instance.needs_players(Mention::Area(view.area).span(&content), view.players.least),
            ],
            _ => Vec::new(),
        };
        let look = TextLook::role(TextRole::Small, ink::INK3);
        for note in notes.iter().rev() {
            text::show(ui, world, &note.styled(&content), look, None);
        }
    }
    if let Some(answer) = chosen {
        let offer = tray::offer(world);
        if let Some(step) = prompt::view(world).map(|prompt| prompt.step) {
            tray::sent(world, step);
        }
        world.write_message(EntranceChoice {
            step: view.step,
            answer,
            offer,
        });
    }
}

const SHEET: Vec2 = Vec2::new(820.0, 380.0);
const SHEET_LEAST: Vec2 = Vec2::new(700.0, 320.0);
