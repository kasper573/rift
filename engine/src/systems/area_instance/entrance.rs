//! The entrance panel: what a player chooses at an entrance to an instanced area, alone or with
//! the party they lead. A player in no party, at an area that may be done alone, never sees it:
//! the entrance's own script already asked, and the copy opens at once.

use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::roster::{InstanceAsk, InstanceDoor};
use super::{AreaInstanceEntry, PlayerCount};
use crate::core::content::Content;
use crate::core::script::{Answer, REFUSED};
use crate::core::units::{MeterPos, Meters, Seconds};
use crate::systems::area::AreaId;
use crate::systems::interact::Interactive;
use crate::systems::mention::Mention;
use crate::systems::player::{self, CharacterAccount, PlayedBy, RequestNeeds, session};
use crate::systems::screen::ClientScreen;
use crate::systems::script::{FlowKey, RiftInstances};
use crate::systems::society::{self, ServerSociety, SocietyAsk};
use crate::systems::text::LineText;
use crate::systems::{GameSettings, hud, movement, stat};

pub(crate) fn register(app: &mut App) {
    use bevy_replication::{AppMessageExt, AppReplicationExt};
    app.replicate_private::<EntranceView>()
        .add_client_message::<EntranceChoice>();
}

pub(crate) fn plugin(app: &mut App) {
    app.add_systems(Update, ask.run_if(in_state(ClientScreen::Area)));
}

/// The turn of an entrance panel a choice answers, so a late choice answers a panel that is
/// already gone.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EntranceStep(pub u32);

/// What the entrance panel shows its player.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct EntranceView {
    pub step: EntranceStep,
    pub area: AreaId,
    pub players: PlayerCount,
    pub lasts: Option<Seconds>,
    pub goals: u32,
    pub party: EntranceParty,
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
    /// Another member holds a place in an open copy of this area.
    Inside,
}

/// The player's choice at the entrance panel.
#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntranceChoice {
    pub step: EntranceStep,
    pub answer: EntranceAnswer,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntranceAnswer {
    GoAlone,
    Gather,
    Cancel,
}

/// Lets `character` in as an entrance's script asks: at once for a player in no party at an
/// area that may be done alone, otherwise through the entrance panel, the script waiting on
/// what they choose. `owner` is what runs the script, the entrance itself where it is a thing in
/// the world.
pub(crate) fn enter(
    world: &mut World,
    character: Entity,
    owner: Option<Entity>,
    entry: AreaInstanceEntry,
) {
    let Some(client) = world.get::<PlayedBy>(character).map(|by| by.client) else {
        refuse(world, entry.flow);
        return;
    };
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
        None if rules.players.least <= 1 => {
            drop(society);
            society::ask(
                world,
                client,
                SocietyAsk::Instance(InstanceAsk::GoAlone(entry)),
            );
            return;
        }
        None => EntranceParty::Alone,
        Some(_) if society.instances().party_inside(parties, &account, area) => {
            EntranceParty::Inside
        }
        Some(party) if party.leader == account => EntranceParty::Leads {
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
    if let Some(earlier) = world
        .get::<EntranceSession>(character)
        .map(|s| s.entry.flow)
    {
        refuse(world, earlier);
    }
    let step = {
        let mut steps = world.resource_mut::<EntranceSteps>();
        steps.0 = steps.0.wrapping_add(1);
        EntranceStep(steps.0)
    };
    let door = door(world, character, owner);
    let view = EntranceView {
        step,
        area,
        players: rules.players,
        lasts: rules.lasts,
        goals: rules.goals.len() as u32,
        party,
    };
    world
        .entity_mut(character)
        .insert((EntranceSession { step, entry, door }, view));
}

/// What players of this world chose at their entrance panels.
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
            let session = world
                .entity_mut(player)
                .take::<EntranceSession>()
                .expect("the session just read");
            world.entity_mut(player).remove::<EntranceView>();
            let at_door = session
                .door
                .is_none_or(|door| stands_at(world, player, door));
            let client = world.get::<PlayedBy>(player).map(|by| by.client);
            let ask = match choice.answer {
                EntranceAnswer::GoAlone if at_door => {
                    Some(InstanceAsk::GoAlone(session.entry.clone()))
                }
                EntranceAnswer::Gather if at_door => Some(InstanceAsk::Gather {
                    entry: session.entry.clone(),
                    door: session.door,
                }),
                _ => None,
            };
            match (ask, client) {
                (Some(ask), Some(client)) => society::ask(world, client, SocietyAsk::Instance(ask)),
                _ => refuse(world, session.entry.flow),
            }
            Ok(())
        },
    );
}

/// Closes the entrance panel of whoever can no longer answer it where they stand: fallen, or
/// out of the entrance's reach.
pub fn hold(world: &mut World, sessions: &mut QueryState<(Entity, &'static EntranceSession)>) {
    let over: Vec<Entity> = sessions
        .iter(world)
        .filter(|(player, session)| {
            stat::is_dead(world, *player)
                || session
                    .door
                    .is_some_and(|door| !stands_at(world, *player, door))
        })
        .map(|(player, _)| player)
        .collect();
    for player in over {
        close(world, player);
    }
}

/// Closes `player`'s entrance panel, if one is open, its script taking the refusal: for one who
/// leaves the world it was opened in, or leaves their character.
pub fn close(world: &mut World, player: Entity) {
    let Ok(mut player) = world.get_entity_mut(player) else {
        return;
    };
    let Some(session) = player.take::<EntranceSession>() else {
        return;
    };
    player.remove::<EntranceView>();
    refuse(world, session.entry.flow);
}

/// The entrance panel the player is shown, for a driver of the client.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ShownInstanceEntrance {
    pub area: String,
    pub least: u32,
    pub most: u32,
    /// Whether Go alone is open.
    pub alone: bool,
    /// Whether Gather the party is offered: to a party's leader alone.
    pub gather: bool,
    /// How many members of the party stand within the player's sight.
    pub nearby: u32,
    /// Whether another member of the party is inside a copy of the area.
    pub party_inside: bool,
}

pub fn shown(world: &World) -> Option<ShownInstanceEntrance> {
    let view = session::my_character(world)?.get::<EntranceView>()?;
    let content = world.resource::<Content>();
    Some(ShownInstanceEntrance {
        area: view.area.get(content).name.clone(),
        least: view.players.least,
        most: view.players.most,
        alone: view.alone_opens(),
        gather: matches!(view.party, EntranceParty::Leads { .. }),
        nearby: match &view.party {
            EntranceParty::Leads { nearby, .. } => nearby.len() as u32,
            _ => 0,
        },
        party_inside: view.party == EntranceParty::Inside,
    })
}

/// The panel's server side: the script waiting on the choice, and where the entrance stands.
#[derive(Component)]
pub struct EntranceSession {
    step: EntranceStep,
    entry: AreaInstanceEntry,
    door: Option<InstanceDoor>,
}

#[derive(Resource, Default)]
pub(crate) struct EntranceSteps(u32);

fn refuse(world: &mut World, flow: FlowKey) {
    world
        .resource_mut::<RiftInstances>()
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

/// Where the entrance stands: the thing whose script asked, where it is one players use, and how
/// far from it they may stand to use it. An entrance that is no thing in the world, such as an
/// item, stands nowhere.
fn door(world: &World, character: Entity, owner: Option<Entity>) -> Option<InstanceDoor> {
    let owner = owner.filter(|owner| *owner != character)?;
    let reach: Meters = world.get::<Interactive>(owner)?.reach;
    Some(InstanceDoor {
        at: movement::position(world, owner)?,
        reach,
    })
}

fn stands_at(world: &World, character: Entity, door: InstanceDoor) -> bool {
    movement::position(world, character).is_some_and(|at| at.distance(door.at) <= door.reach)
}

/// Shows the entrance panel the server opened for the player, once for each turn of it.
fn ask(world: &mut World, mut asked: Local<Option<EntranceStep>>) {
    let Some(view) = session::my_character(world).and_then(|me| me.get::<EntranceView>().cloned())
    else {
        return;
    };
    if *asked == Some(view.step) {
        return;
    }
    *asked = Some(view.step);
    let content = world.resource::<Content>().clone();
    let words = world.resource::<GameSettings>().words.clone();
    let area = Mention::Area(view.area).span(&content);
    let players = view.players;
    let needs = LineText::plain(words.instance_needs_players.as_str())
        .filling("{area}", area.clone())
        .filling("{least}", players.least.to_string());
    let mut chips = vec![match (players.least, players.most) {
        (1, 1) => "1 player".to_owned(),
        (least, most) if least == most => format!("{least} players"),
        (least, most) => format!("{least} to {most} players"),
    }];
    if let Some(lasts) = view.lasts {
        chips.push(format!("{} min limit", (lasts.0 / 60.0).ceil() as u32));
    }
    chips.push(match view.goals {
        1 => "1 goal".to_owned(),
        goals => format!("{goals} goals"),
    });
    let alone = |label: String, open_note: Option<&str>| {
        let option = hud::ChoiceOption {
            label,
            note: if view.alone_opens() {
                open_note.map(LineText::from)
            } else {
                Some(needs.clone())
            },
            open: view.alone_opens(),
        };
        (option, EntranceAnswer::GoAlone)
    };
    let (body, options, after, cancel, focus) = match &view.party {
        EntranceParty::Alone => (
            Vec::new(),
            vec![alone(words.instance_go_alone.clone(), None)],
            vec![LineText::plain(
                words
                    .instance_form_party
                    .replace("{least}", &players.least.to_string()),
            )],
            "Close",
            None,
        ),
        EntranceParty::Leads { size, nearby } => {
            let mut said = format!("You lead a party of {size}.");
            if !nearby.is_empty() {
                let names = LineText::listed(
                    nearby
                        .iter()
                        .map(|name| LineText::from(name.as_str()))
                        .collect(),
                );
                let stand = if nearby.len() == 1 { "stands" } else { "stand" };
                said = format!("{said} {} {stand} near you.", names.plain_words());
            }
            let gather = hud::ChoiceOption {
                label: format!("{} · {} nearby", words.instance_gather, nearby.len()),
                note: Some("Choose who comes. Each member answers for themselves.".into()),
                open: true,
            };
            let gather = (gather, EntranceAnswer::Gather);
            (
                vec![LineText::plain(said)],
                vec![
                    alone(
                        words.instance_go_alone.clone(),
                        Some("Your party keeps playing where it is."),
                    ),
                    gather,
                ],
                Vec::new(),
                "Cancel",
                Some(1),
            )
        }
        EntranceParty::Member { leader } => (
            Vec::new(),
            vec![alone(words.instance_go_alone.clone(), None)],
            vec![LineText::plain(
                words.instance_leader_only.replace("{leader}", leader),
            )],
            "Cancel",
            view.alone_opens().then_some(0),
        ),
        EntranceParty::Inside => (
            vec![
                LineText::plain(words.instance_already_inside.as_str())
                    .filling("{area}", area.clone()),
            ],
            vec![alone(
                format!("{}, in a copy of your own", words.instance_go_alone),
                None,
            )],
            Vec::new(),
            "Close",
            None,
        ),
    };
    let step = view.step;
    let (options, answers): (Vec<hud::ChoiceOption>, Vec<EntranceAnswer>) =
        options.into_iter().unzip();
    hud::choose(
        world,
        hud::Choose {
            title: Mention::Area(view.area).line(&content),
            chips,
            body,
            options,
            after,
            cancel: cancel.to_owned(),
            focus,
            answer: Box::new(move |world, chosen| {
                let answer = chosen
                    .and_then(|index| answers.get(index).copied())
                    .unwrap_or(EntranceAnswer::Cancel);
                world.write_message(EntranceChoice { step, answer });
            }),
        },
    );
}
