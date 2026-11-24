//! A leader's call to their party: who of the members within sight of the leader joins, who the
//! leader picks of them, and the instance that opens for them once the leader enters. A call is
//! society's, as parties are, and lives from Gather the party until Enter or until it ends. It
//! is never saved: nobody has moved while it is open, so a server that stops loses nothing.

use std::collections::BTreeMap;

use bevy_app::App;
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use super::{AreaInstanceEntry, PlayerCount, answer as answer_flow, open, step_in};
use crate::core::content::Content;
use crate::core::math::Pos;
use crate::core::script::{NEXT, REFUSED};
use crate::core::time::UnixMillis;
use crate::core::units::{MeterPos, Meters};
use crate::systems::account::AccountId;
use crate::systems::area::AreaId;
use crate::systems::history::HistoryTopic;
use crate::systems::mention::Mention;
use crate::systems::messages;
use crate::systems::notification::{self, Notification};
use crate::systems::player::{self, PlayedBy, RequestNeeds};
use crate::systems::society::{self, AccountNumber, Settling, Society, SocietyAsk, SocietyMember};
use crate::systems::worlds::WorldName;
use crate::systems::{GameSettings, stat};
use schemars::JsonSchema;

pub fn register(app: &mut App) {
    use bevy_replication::AppMessageExt;
    app.add_client_message::<CallRequest>()
        .add_client_message::<CallReply>();
}

/// What a world asks of society about instances, for one of its players.
#[derive(Clone, Debug, PartialEq)]
pub enum InstanceAsk {
    /// An instance of their own.
    GoAlone(AreaInstanceEntry),
    /// A call to the party they lead, made at `door` where the entrance is a thing in the world.
    Gather {
        entry: AreaInstanceEntry,
        door: Option<InstanceDoor>,
    },
    Call(CallRequest),
    Reply(CallReply),
}

/// Where an entrance stands in the world its script runs in, and how far from it its player may
/// stand to use it. A leader who walks farther away ends their call.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InstanceDoor {
    pub at: Pos<Meters>,
    pub reach: Meters,
}

/// A leader's call to their party, open from Gather the party until Enter or until it ends.
#[derive(Clone, Debug, PartialEq)]
pub struct InstanceCall {
    pub entry: AreaInstanceEntry,
    pub door: Option<InstanceDoor>,
    pub ends: UnixMillis,
    /// Whether the leader lies fallen, as the worlds last conferred.
    leader_fallen: bool,
    /// Every member of the party but its leader, in the order they joined the party.
    members: Vec<CallMember>,
}

/// A member asked to answer a call.
#[derive(Clone, Debug, PartialEq)]
pub struct CallMember {
    pub account: AccountId,
    pub answer: CallAnswer,
    /// Where they stand toward the leader, as the worlds last conferred.
    standing: CallStanding,
    /// Whether they were shown the call: every member who came within sight of the leader.
    called: bool,
}

/// A member's answer to a call. Only a member who joined can be picked to go.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallAnswer {
    Deciding,
    Declined,
    Joined { picked: bool },
}

/// What a leader asks of their call.
#[derive(JsonSchema, Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallRequest {
    Pick(AccountNumber),
    Unpick(AccountNumber),
    Enter,
    CallOff,
}

/// What a member answers. They may answer again until the party enters.
#[derive(JsonSchema, Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallReply {
    Join,
    Decline,
}

/// How someone stands in a call, as the roster and the card show it.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum RosterRowShown {
    /// The leader, who goes while they stand.
    Leading,
    Going,
    Benched,
    Deciding {
        ends: UnixMillis,
    },
    NotGoing,
    TooFar {
        area: AreaId,
    },
    /// Their player is away from their character.
    Away,
    Fallen,
}

/// A call as one of its players sees it: the leader the whole roster, a member their own row.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct InstanceCallShown {
    pub leader: SocietyMember,
    pub area: AreaId,
    pub players: PlayerCount,
    pub ends: UnixMillis,
    /// Whether the player it is shown to leads it.
    pub leads: bool,
    /// The leader first, then the members in the order they joined the party; to a member, only
    /// their own row.
    pub rows: Vec<CallRowShown>,
    /// How many slots are taken: the leader and everyone picked.
    pub taken: u32,
    /// How many would go if the leader entered now.
    pub ready: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CallRowShown {
    pub character: SocietyMember,
    pub shown: RosterRowShown,
    /// The leader's row reads as joined and picked.
    pub answer: CallAnswer,
}

/// Every call that is open, by its leader.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InstanceCalls(BTreeMap<AccountId, InstanceCall>);

impl InstanceCalls {
    /// The call `leader` made.
    pub fn led_by(&self, leader: &AccountId) -> Option<&InstanceCall> {
        self.0.get(leader)
    }

    /// The call `member` is asked to answer, and its leader.
    pub fn asking(&self, member: &AccountId) -> Option<(&AccountId, &InstanceCall)> {
        self.0
            .iter()
            .find(|(_, call)| call.member(member).is_some())
    }
}

impl InstanceCall {
    /// How many go as it stands: the leader, and everyone picked.
    pub fn taken(&self) -> u32 {
        let picked = self
            .members
            .iter()
            .filter(|member| member.answer == CallAnswer::Joined { picked: true })
            .count();
        1 + picked as u32
    }

    /// Who would go besides the leader if the leader entered now: everyone picked who stands
    /// within sight. None while the leader lies fallen.
    pub fn going(&self) -> Option<Vec<AccountId>> {
        if self.leader_fallen {
            return None;
        }
        let picked = self
            .members
            .iter()
            .filter(|member| {
                member.answer == CallAnswer::Joined { picked: true }
                    && member.standing == CallStanding::InSight
            })
            .map(|member| member.account.clone())
            .collect();
        Some(picked)
    }

    /// How many would go if the leader entered now.
    pub fn ready(&self) -> u32 {
        self.going().map_or(0, |going| 1 + going.len() as u32)
    }

    fn member(&self, account: &AccountId) -> Option<&CallMember> {
        self.members
            .iter()
            .find(|member| member.account == *account)
    }

    fn member_mut(&mut self, account: &AccountId) -> Option<&mut CallMember> {
        self.members
            .iter_mut()
            .find(|member| member.account == *account)
    }

    fn players(&self, content: &Content) -> PlayerCount {
        self.entry
            .to
            .area
            .get(content)
            .instance
            .as_ref()
            .map_or(PlayerCount { least: 1, most: 1 }, |rules| rules.players)
    }

    fn row(&self, member: &CallMember) -> RosterRowShown {
        match member.standing {
            CallStanding::Fallen => RosterRowShown::Fallen,
            CallStanding::TooFar(Some(area)) => RosterRowShown::TooFar { area },
            CallStanding::TooFar(None) => RosterRowShown::Away,
            CallStanding::InSight => match member.answer {
                CallAnswer::Joined { picked: true } => RosterRowShown::Going,
                CallAnswer::Joined { picked: false } => RosterRowShown::Benched,
                CallAnswer::Declined => RosterRowShown::NotGoing,
                CallAnswer::Deciding => RosterRowShown::Deciding { ends: self.ends },
            },
        }
    }
}

impl InstanceCallShown {
    /// The call `account` leads or is asked to answer, once they were shown it.
    pub(crate) fn of(
        society: &Society,
        account: &AccountId,
        content: &Content,
    ) -> Option<InstanceCallShown> {
        let calls = society.instances().calls();
        let (leader, call) = match calls.led_by(account) {
            Some(call) => (account, call),
            None => calls
                .asking(account)
                .filter(|(_, call)| call.member(account).is_some_and(|member| member.called))?,
        };
        let leads = leader == account;
        let row = |member: &CallMember| {
            Some(CallRowShown {
                character: society.member(&member.account)?,
                shown: call.row(member),
                answer: member.answer,
            })
        };
        let rows = if leads {
            let leader_row = CallRowShown {
                character: society.member(leader)?,
                shown: if call.leader_fallen {
                    RosterRowShown::Fallen
                } else {
                    RosterRowShown::Leading
                },
                answer: CallAnswer::Joined { picked: true },
            };
            std::iter::once(leader_row)
                .chain(call.members.iter().filter_map(row))
                .collect()
        } else {
            call.member(account).and_then(row).into_iter().collect()
        };
        Some(InstanceCallShown {
            leader: society.member(leader)?,
            area: call.entry.to.area,
            players: call.players(content),
            ends: call.ends,
            leads,
            rows,
            taken: call.taken(),
            ready: call.ready(),
        })
    }
}

/// Hands what players of this world asked of calls to society.
pub fn requests(world: &mut World) {
    let client = |world: &World, player: Entity| world.get::<PlayedBy>(player).map(|by| by.client);
    player::player_requests::<CallRequest>(world, RequestNeeds::Nothing, |world, player, asked| {
        if let Some(client) = client(world, player) {
            society::ask(
                world,
                client,
                SocietyAsk::Instance(InstanceAsk::Call(asked)),
            );
        }
        Ok(())
    });
    player::player_requests::<CallReply>(world, RequestNeeds::Nothing, |world, player, reply| {
        if let Some(client) = client(world, player) {
            society::ask(
                world,
                client,
                SocietyAsk::Instance(InstanceAsk::Reply(reply)),
            );
        }
        Ok(())
    });
}

/// Does what `by` asked of instances.
pub(crate) fn settle(settling: &mut Settling, content: &Content, by: &AccountId, ask: InstanceAsk) {
    match ask {
        InstanceAsk::GoAlone(entry) => super::go_alone(settling, content, by, entry),
        InstanceAsk::Gather { entry, door } => gather(settling, content, by, entry, door),
        InstanceAsk::Call(request) => ask_call(settling, content, by, request),
        InstanceAsk::Reply(reply) => answer(settling, content, by, reply),
    }
}

/// Keeps every call as the worlds confer: one whose leader no longer leads or is away from
/// their character, who left the entrance, or whose time is up, ends; the rest follow the
/// party's members and where they stand.
pub(crate) fn keep(settling: &mut Settling, content: &Content) {
    let leaders: Vec<AccountId> = settling.society.instances.calls.0.keys().cloned().collect();
    for leader in leaders {
        let Some(call) = settling.society.instances.calls.0.get(&leader) else {
            continue;
        };
        let leads = settling.society.parties.leads(&leader);
        let at = place(settling, &leader);
        let ended = if !leads || at.is_none() {
            Some(CallEnd::LostLead)
        } else if settling.now >= call.ends {
            Some(CallEnd::RanOut)
        } else if !at_entrance(call, at) {
            Some(CallEnd::LeftDoor)
        } else {
            None
        };
        match ended {
            Some(why) => end(settling, content, &leader, why),
            None => follow(settling, content, &leader),
        }
    }
}

/// Why a call ended without its party going in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CallEnd {
    CalledOff,
    RanOut,
    LostLead,
    LeftDoor,
}

/// Where a member stands toward their leader.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CallStanding {
    InSight,
    Fallen,
    /// Out of the leader's sight, in this area; in none while their player is away.
    TooFar(Option<AreaId>),
}

/// Where a character stood as the worlds last conferred: its world and its spot.
type Standing = (WorldName, Pos<Meters>);

fn gather(
    settling: &mut Settling,
    content: &Content,
    by: &AccountId,
    entry: AreaInstanceEntry,
    door: Option<InstanceDoor>,
) {
    let leads = settling.society.parties.leads(by);
    let (true, Some(rules)) = (leads, content.built::<GameSettings>().party) else {
        answer_flow(settling, &entry, REFUSED);
        return;
    };
    if settling.society.instances.calls.0.contains_key(by) {
        end(settling, content, by, CallEnd::CalledOff);
    }
    let call = InstanceCall {
        entry,
        door,
        ends: settling.now.after(rules.call_lasts),
        leader_fallen: false,
        members: Vec::new(),
    };
    let sent = messages::of(content)
        .instance
        .call_sent(Mention::Area(call.entry.to.area).span(content));
    settling.society.instances.calls.0.insert(by.clone(), call);
    settling.society.changed(by);
    settling.tell(by, Notification::feed(HistoryTopic::Notification, sent));
    follow(settling, content, by);
}

fn ask_call(settling: &mut Settling, content: &Content, by: &AccountId, request: CallRequest) {
    let (picks, number) = match request {
        CallRequest::Pick(number) => (true, number),
        CallRequest::Unpick(number) => (false, number),
        CallRequest::Enter => return enter(settling, content, by),
        CallRequest::CallOff => return end(settling, content, by, CallEnd::CalledOff),
    };
    let Some(member) = settling.society.accounts().numbered(number).cloned() else {
        return;
    };
    let Some(call) = settling.society.instances.calls.0.get_mut(by) else {
        return;
    };
    let room = call.taken() < call.players(content).most;
    let Some(asked) = call.member_mut(&member) else {
        return;
    };
    let picked = match (picks, asked.answer, asked.standing) {
        (true, CallAnswer::Joined { picked: false }, CallStanding::InSight) if room => true,
        (false, CallAnswer::Joined { picked: true }, _) => false,
        _ => return,
    };
    asked.answer = CallAnswer::Joined { picked };
    settling.society.changed(by);
    settling.society.changed(&member);
}

/// Takes a member's answer to the call they were shown. Joining takes a free slot at once while
/// they stand within sight, so the leader picks only when more join than fit.
fn answer(settling: &mut Settling, content: &Content, by: &AccountId, reply: CallReply) {
    let Some(leader) = settling
        .society
        .instances
        .calls
        .asking(by)
        .map(|(leader, _)| leader.clone())
    else {
        return;
    };
    let Some(call) = settling.society.instances.calls.0.get_mut(&leader) else {
        return;
    };
    let room = call.taken() < call.players(content).most;
    let Some(member) = call.member_mut(by).filter(|member| member.called) else {
        return;
    };
    member.answer = match (reply, member.answer) {
        (CallReply::Join, CallAnswer::Joined { .. })
        | (CallReply::Decline, CallAnswer::Declined) => {
            return;
        }
        (CallReply::Join, _) => CallAnswer::Joined {
            picked: room && member.standing == CallStanding::InSight,
        },
        (CallReply::Decline, _) => CallAnswer::Declined,
    };
    settling.society.changed(&leader);
    settling.society.changed(by);
}

/// Opens the instance for everyone the leader takes, as long as the leader stands and enough would
/// go: each goes in, and the rest who joined are told the party went without them.
fn enter(settling: &mut Settling, content: &Content, leader: &AccountId) {
    let messages = &messages::of(content).instance;
    follow(settling, content, leader);
    let Some(call) = settling.society.instances.calls.0.get(leader).cloned() else {
        return;
    };
    let Some(going) = call.going() else {
        return;
    };
    let players = call.players(content);
    let area = Mention::Area(call.entry.to.area).span(content);
    let going: Vec<AccountId> = std::iter::once(leader.clone()).chain(going).collect();
    if (going.len() as u32) < players.least {
        let why = messages.needs_players(area, players.least);
        settling.refuse(leader, why.plain_words(content));
        return;
    }
    let run = match open(settling, content, &call.entry, leader, going.clone()) {
        Ok(run) => run,
        Err(why) => {
            settling.refuse(leader, why.plain_words(content));
            return;
        }
    };
    settling.society.instances.calls.0.remove(leader);
    for account in &going {
        if let Some((world, character)) = settling.reach(account) {
            step_in(world, character, call.entry.to, run);
        }
    }
    answer_flow(settling, &call.entry, NEXT);
    let leader_name = settling.name(leader);
    for member in &call.members {
        settling.society.changed(&member.account);
        let joined = matches!(member.answer, CallAnswer::Joined { .. });
        if going.contains(&member.account) || !joined {
            continue;
        }
        let said = match member.standing {
            CallStanding::TooFar(Some(_)) => messages.went_too_far(area.clone(), &leader_name),
            _ => messages.went_without(area.clone()),
        };
        settling.tell(
            &member.account,
            Notification::feed(HistoryTopic::Notification, said),
        );
    }
    settling.society.changed(leader);
}

/// Ends the call `leader` made without anyone going in: every member who was shown it is told
/// why, and the entrance's script takes its refusal.
fn end(settling: &mut Settling, content: &Content, leader: &AccountId, why: CallEnd) {
    let Some(call) = settling.society.instances.calls.0.remove(leader) else {
        return;
    };
    let messages = &messages::of(content).instance;
    let area = Mention::Area(call.entry.to.area).span(content);
    let leader_name = settling.name(leader);
    let said = match why {
        CallEnd::CalledOff => messages.called_off(area, leader_name),
        CallEnd::RanOut => messages.call_ran_out(area),
        CallEnd::LostLead => messages.call_lost_lead(area, leader_name),
        CallEnd::LeftDoor => messages.call_left_door(leader_name),
    };
    for member in &call.members {
        settling.society.changed(&member.account);
        if member.called {
            settling.tell(
                &member.account,
                Notification::feed(HistoryTopic::Notification, said.clone()),
            );
        }
    }
    settling.society.changed(leader);
    if why != CallEnd::CalledOff
        && call.entry.told
        && let Some((world, character)) = settling.reach(leader)
    {
        notification::error(world, character, said);
    }
    answer_flow(settling, &call.entry, REFUSED);
}

/// Follows the party of the call `leader` made: who joined the party since is asked too, who
/// left it is not, and where each member stands toward the leader is noted, showing the call
/// to whoever came within sight of them.
fn follow(settling: &mut Settling, content: &Content, leader: &AccountId) {
    let sight = content.built::<GameSettings>().sight;
    let in_party: Vec<AccountId> = settling
        .society
        .parties
        .of_account(leader)
        .map(|party| party.others(leader).cloned().collect())
        .unwrap_or_default();
    let from = place(settling, leader);
    let leader_fallen = fallen(settling, leader);
    let standing: Vec<CallStanding> = in_party
        .iter()
        .map(|member| stand(settling, from, member, sight))
        .collect();
    let Some(call) = settling.society.instances.calls.0.get_mut(leader) else {
        return;
    };
    let before = call.clone();
    call.members = in_party
        .into_iter()
        .zip(standing)
        .map(|(account, standing)| {
            let (answer, called) = before
                .member(&account)
                .map_or((CallAnswer::Deciding, false), |known| {
                    (known.answer, known.called)
                });
            CallMember {
                called: called || standing == CallStanding::InSight,
                account,
                answer,
                standing,
            }
        })
        .collect();
    call.leader_fallen = leader_fallen;
    if *call == before {
        return;
    }
    let changed: Vec<AccountId> = before
        .members
        .iter()
        .chain(&call.members)
        .map(|member| member.account.clone())
        .chain(std::iter::once(leader.clone()))
        .collect();
    for account in changed {
        settling.society.changed(&account);
    }
}

/// Where `account`'s character stood as the worlds last conferred. None while their player is
/// away from it.
fn place(settling: &Settling, account: &AccountId) -> Option<Standing> {
    let enrolled = settling
        .roster
        .account(account)
        .filter(|enrolled| !enrolled.away)?;
    let at = settling.roster.place_of(enrolled.client)?.at;
    let world = WorldName {
        area: enrolled.area,
        instance: enrolled.instance,
    };
    Some((world, at))
}

fn stand(
    settling: &mut Settling,
    leader: Option<Standing>,
    member: &AccountId,
    sight: Meters,
) -> CallStanding {
    let Some((world, at)) = place(settling, member) else {
        return CallStanding::TooFar(None);
    };
    let near = leader.is_some_and(|(leader_world, leader_at)| {
        leader_world == world && at.distance(leader_at) <= sight
    });
    if !near {
        CallStanding::TooFar(Some(world.area))
    } else if fallen(settling, member) {
        CallStanding::Fallen
    } else {
        CallStanding::InSight
    }
}

fn fallen(settling: &mut Settling, account: &AccountId) -> bool {
    settling
        .reach(account)
        .is_some_and(|(world, character)| stat::is_dead(world, character))
}

/// Whether the leader, standing where `at` says, is still at the entrance: in the world its
/// script runs in, and within reach of it where it is a thing in that world.
fn at_entrance(call: &InstanceCall, at: Option<Standing>) -> bool {
    at.is_some_and(|(world, at)| {
        world == call.entry.asked_in
            && call
                .door
                .is_none_or(|door| at.distance(door.at) <= door.reach)
    })
}
