//! Parties: a handful of players who chose to play together for now. A party is of the whole
//! server and not of a world, has one leader, and shares drops, experience and the kills its
//! quests count.

pub mod frames;
pub mod page;
pub mod query;
pub mod share;

use crate::core::script::Vocabulary;
use std::collections::{BTreeMap, HashMap};

use bevy_app::App;
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::Content;
use crate::core::store::{Record, RecordId};
use crate::core::time::UnixMillis;
use crate::core::units::{Meters, Seconds};
use crate::systems::GameSettings;
use crate::systems::account::AccountId;
use crate::systems::area::AreaId;
use crate::systems::chat::{self, ChatChannel, ChatChannelSource, ChatSaid, ChatSpeaker};
use crate::systems::history::HistoryTopic;
use crate::systems::messages::{self, messages};
use crate::systems::notification::Notification;
use crate::systems::player::RequestNeeds;
use crate::systems::player::{self, ClientId, PlayedBy};
use crate::systems::social_request::{
    self, RequestUnaccepted, RequestUnsent, SocialRequest, SocialRequestAsk, SocialRequestKind,
    SocialRequestSource, SocialRequestWords,
};
use crate::systems::society::{
    self, AccountNumber, Settling, Society, SocietyAsk, SocietyMember, SocietyView,
};

messages! {
    /// What the game says of parties.
    pub struct PartyMessages {
        /// What the plate of a character says whose player is gone.
        away,
        /// What an invitation to a party from `{name}` says to the player invited.
        invite_asks(name: Text),
        /// What an invitation to a party says to the player who invited `{name}`.
        invite_asked(name: Text),
        /// What a player is told as their invitation to a party is sent to `{name}`.
        invite_sent(name: Text),
        /// What a player is told as `{name}` denies their invitation to a party.
        invite_denied(name: Text),
        /// What a player is told as their invitation to `{name}` runs out unanswered.
        invite_lapsed(name: Text),
        /// What a player is told inviting someone to a party that has no room.
        full,
        /// What a player is told inviting `{name}`, who is in a party.
        in_a_party(name: Text),
        /// What the members of a party are told as `{name}` joins it.
        joined(name: Text),
        /// What the members of a party are told as `{name}` leaves it.
        left(name: Text),
        /// What the members of a party are told as its leader removes `{name}` from it.
        removed(name: Text),
        /// What a player is told as the leader of their party removes them from it.
        you_were_removed,
        /// What the members of a party are told as its leader `{name}` ends it.
        disbanded(name: Text),
        /// What the members of a party are told as `{name}` comes to lead it.
        leads(name: Text),
        /// What the party's chat says to a player as they join the party `{name}` leads.
        you_joined(name: Text),
        /// What chat says to a player as they leave their party.
        you_left,
        /// The name of the Party page, and the heading of the party's frames on the HUD.
        title,
        /// The heading of the party's frames on the HUD while they are folded away, `{count}` the members they list.
        tracker_title_folded(count: Number),
        /// What a screen reader names the party's heading on the HUD while its frames are folded away.
        tracker_show,
        /// What a screen reader names the party's heading on the HUD while its frames are shown.
        tracker_hide,
        /// What a member's frame says of their health, `{percent}` the share they hold.
        health_percent(percent: Number),
        /// What a member's frame says of a member whose player is gone, `{time}` until they leave the party.
        away_leaves(time: Text),
        /// What a member's frame says of a member whose player is gone.
        away_stays,
        /// How a screen reader names the leader of a party, `{name}` their name.
        leader_named(name: Text),
        /// The hint, beside its key, that chooses a member on the Party page.
        hint_choose,
        /// The heading over the party of a player who is in none.
        your_party,
        /// What the page says to a player who is in no party.
        no_party,
        /// The button that makes a party.
        create,
        /// The heading over the invitations to a party the player is asked.
        invitations_heading,
        /// What an invitation says: `{name}` invites, `{left}` is how long it waits.
        invitation_line(left: Text, name: Text),
        /// The heading over the player's party: `{count}` members of the most it holds, `{size}`.
        party_of_size(count: Number, size: Number),
        /// The mark that says the player leads their party.
        you_lead,
        /// The button that hands the lead of the party to the member chosen.
        make_leader,
        /// The button that removes the member chosen from the party.
        remove_member,
        /// What the page tells a member who does not lead what members may do.
        members_note,
        /// The button that leaves the party.
        leave,
        /// The button that ends the party, for its leader.
        disband,
        /// What a member's row says of their class `{class}` and level `{level}`.
        class_level(class: Text, level: Number),
        /// The heading over the search for who to invite.
        invite_heading,
        /// The heading over the friends who play right now.
        friends_online,
        /// What the page says when no friend plays right now.
        none_online,
        /// What the page says of a friend who is in a party already.
        in_another_party,
        /// The mark on a found player who is in the player's party.
        in_the_party,
        /// The mark on a found player who has an invitation waiting, `{left}` how long it waits.
        invited(left: Text),
        /// The button that invites a player to the party.
        invite,
        /// The action that invites a player a page lists to the party.
        invite_to_party,
        /// The title of the question whether to end the party.
        disband_title,
        /// What the question whether to end the party tells the leader.
        disband_body,
        /// The button that confirms ending the party.
        disband_confirm,
        /// How a screen reader names a member's row or frame: `{name}` is the member, `{state}` how they stand.
        row_named(name: Text, state: Text),
        /// How a screen reader names a button: `{does}` is what it does, `{name}` the member or player it does it to.
        button_for(does: Text, name: Text),
    }
}

pub fn register(app: &mut App) {
    use bevy_replication::AppMessageExt;
    app.add_client_message::<PartyRequest>();
}

/// How a game has parties work. A game without it has none.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartySettings {
    /// The most members a party holds.
    pub size: u32,
    /// How long an invitation waits for its answer.
    pub invite_lasts: Seconds,
    /// How long a leader's call to gather the party for an instance stays open.
    pub call_lasts: Seconds,
    /// How long a member whose player is gone stays in the party.
    pub absent_for: Seconds,
    pub experience: SharedExperience,
}

/// How a party shares the experience of a kill.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedExperience {
    pub split: ExperienceSplit,
    /// How far from where a foe fell a member still shares in it.
    pub range: Meters,
    /// Percent added to a kill's worth for each member who shares in it past the first.
    pub bonus: f32,
}

/// How a kill's worth is divided among the members who share in it.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExperienceSplit {
    /// In equal parts.
    Even,
    /// In parts as large as each member's level.
    ByLevel,
    /// Not at all: each member gets all of it.
    Whole,
}

#[derive(
    JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
pub struct PartyId(pub u64);

/// A party: its leader, and its members in the order they joined.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CharacterParty {
    pub id: PartyId,
    pub leader: AccountId,
    pub members: Vec<PartyMember>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PartyMember {
    pub account: AccountId,
    pub joined: UnixMillis,
}

impl Record for CharacterParty {
    const KIND: &'static str = "party";

    fn id(&self) -> RecordId {
        record_id(self.id)
    }
}

impl CharacterParty {
    pub fn holds(&self, account: &AccountId) -> bool {
        self.members.iter().any(|member| member.account == *account)
    }

    pub fn accounts(&self) -> impl Iterator<Item = &AccountId> {
        self.members.iter().map(|member| &member.account)
    }

    /// Every member but `account`, in the order they joined.
    pub fn others<'a>(&'a self, account: &'a AccountId) -> impl Iterator<Item = &'a AccountId> {
        self.accounts().filter(move |member| *member != account)
    }
}

/// Every party of the server.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CharacterParties {
    parties: BTreeMap<PartyId, CharacterParty>,
    of: HashMap<AccountId, PartyId>,
    /// Since when each member's player has been gone.
    absent: HashMap<AccountId, UnixMillis>,
    dealt: u64,
}

impl CharacterParties {
    pub fn of(saved: impl IntoIterator<Item = CharacterParty>) -> CharacterParties {
        let mut parties = CharacterParties::default();
        for party in saved {
            parties.dealt = parties.dealt.max(party.id.0);
            for account in party.accounts() {
                parties.of.insert(account.clone(), party.id);
            }
            parties.parties.insert(party.id, party);
        }
        parties
    }

    /// The party `account` is in.
    pub fn of_account(&self, account: &AccountId) -> Option<&CharacterParty> {
        self.parties.get(self.of.get(account)?)
    }

    /// Whether there is a party `id`.
    pub fn holds(&self, id: PartyId) -> bool {
        self.parties.contains_key(&id)
    }

    /// Whether the two are in one party.
    pub fn together(&self, one: &AccountId, other: &AccountId) -> bool {
        self.of
            .get(one)
            .is_some_and(|party| self.of.get(other) == Some(party))
    }

    /// Whether `account` leads the party they are in.
    pub fn leads(&self, account: &AccountId) -> bool {
        self.of_account(account)
            .is_some_and(|party| party.leader == *account)
    }

    /// Whether `account` is in a party that `other` is not in.
    pub fn in_another(&self, account: &AccountId, other: &AccountId) -> bool {
        self.of_account(account).is_some() && !self.together(account, other)
    }

    /// Since when the player of `account`, a member of a party, has been gone.
    pub fn absent_since(&self, account: &AccountId) -> Option<UnixMillis> {
        self.absent.get(account).copied()
    }

    fn create(&mut self, leader: &AccountId, now: UnixMillis) -> PartyId {
        self.dealt += 1;
        let id = PartyId(self.dealt);
        let party = CharacterParty {
            id,
            leader: leader.clone(),
            members: vec![PartyMember {
                account: leader.clone(),
                joined: now,
            }],
        };
        self.of.insert(leader.clone(), id);
        self.parties.insert(id, party);
        id
    }
}

/// What a player asks of parties beyond an invitation.
#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartyRequest {
    /// A party of their own, with them as its leader.
    Create,
    Invite(AccountNumber),
    Leave,
    /// That a member be out of the party they lead.
    Remove(AccountNumber),
    /// That a member lead the party they lead.
    Lead(AccountNumber),
    /// That the party they lead be over.
    Disband,
}

/// What a member of a party sees of it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PartyShown {
    pub leader: AccountNumber,
    /// Its members in the order they joined, the player it is shown to among them.
    pub members: Vec<PartyMemberRow>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PartyMemberRow {
    pub character: SocietyMember,
    /// The client their character is played as, while it stands in a world.
    pub client: Option<ClientId>,
    /// The area they play in, while they are at their character.
    pub area: Option<AreaId>,
    /// When they leave the party unless they are back by then, while their player is gone.
    pub leaves_at: Option<UnixMillis>,
}

impl PartyShown {
    /// The clients playing its members' characters.
    pub fn clients(&self) -> impl Iterator<Item = ClientId> {
        self.members.iter().filter_map(|member| member.client)
    }

    /// The party of `account` as its members see it, in a game that has parties.
    pub(crate) fn of(
        society: &Society,
        account: &AccountId,
        settings: &GameSettings,
        client_of: impl Fn(&AccountId) -> Option<ClientId>,
    ) -> Option<PartyShown> {
        let rules = settings.party.as_ref()?;
        let party = society.parties().of_account(account)?;
        Some(PartyShown {
            leader: society.accounts().number(&party.leader)?,
            members: party
                .accounts()
                .filter_map(|member| {
                    Some(PartyMemberRow {
                        character: society.member(member)?,
                        client: client_of(member),
                        area: society.playing_in(member),
                        leaves_at: society
                            .parties()
                            .absent_since(member)
                            .map(|since| since.after(rules.absent_for)),
                    })
                })
                .collect(),
        })
    }
}

/// The clients playing the characters of the party `character` is in, its own among them: none
/// for a character in no party.
pub fn companions(world: &World, character: Entity) -> Vec<ClientId> {
    world
        .get::<SocietyView>(character)
        .and_then(|view| view.party.as_ref())
        .map(|party| party.clients().collect())
        .unwrap_or_default()
}

/// Whether the player of `character` is in a party.
pub fn in_one(world: &World, character: Entity) -> bool {
    world
        .get::<SocietyView>(character)
        .is_some_and(|view| view.party.is_some())
}

/// The party's chat: its members are in it, and a line reaches each of them wherever they play.
pub fn chat_channel() -> ChatChannelSource {
    ChatChannelSource {
        channel: ChatChannel::Party,
        open_to: in_one,
        send: |world, speaker, said| {
            if let Some(client) = world.get::<PlayedBy>(speaker).map(|by| by.client) {
                society::ask(world, client, SocietyAsk::PartyLine(said));
            }
        },
    }
}

/// An invitation to a party as a kind of request: it needs the player invited at their
/// character, and its inviter is told how it went.
pub fn request_source() -> SocialRequestSource {
    SocialRequestSource {
        kind: SocialRequestKind::PartyInvitation,
        lasts: |settings| settings.party.map(|party| party.invite_lasts),
        reaches_offline: false,
        words: SocialRequestWords {
            asks: |messages, name| messages.party.invite_asks(name),
            asked: |messages, name| messages.party.invite_asked(name),
            sent: |messages, name| messages.party.invite_sent(name),
            accepted: None,
            denied: Some(|messages, name| messages.party.invite_denied(name)),
            lapsed: Some(|messages, name| messages.party.invite_lapsed(name)),
        },
        may_send,
        accepted,
        stands: |_, _| true,
    }
}

/// Hands what players of this world asked of parties to society.
pub fn requests(world: &mut World) {
    player::player_requests::<PartyRequest>(
        world,
        RequestNeeds::Nothing,
        |world, player, request| {
            let Some(client) = world.get::<PlayedBy>(player).map(|by| by.client) else {
                return Ok(());
            };
            let ask = match request {
                PartyRequest::Invite(of) => SocietyAsk::Request(SocialRequestAsk::Ask {
                    kind: SocialRequestKind::PartyInvitation,
                    of,
                    about: None,
                }),
                other => SocietyAsk::Party(other),
            };
            society::ask(world, client, ask);
            Ok(())
        },
    );
}

/// Does what `by` asked of parties, under the game `content` is of.
pub(crate) fn settle(
    settling: &mut Settling,
    content: &Content,
    by: &AccountId,
    ask: PartyRequest,
) {
    if content.built::<GameSettings>().party.is_none() {
        return;
    }
    let words = &messages::of(content).party;
    let party = settling.society.parties.of_account(by).cloned();
    let leads = settling.society.parties.leads(by);
    let member = |settling: &Settling, number: AccountNumber| {
        let account = settling.society.accounts().numbered(number)?.clone();
        let party = party.as_ref()?;
        (account != *by && party.holds(&account)).then_some(account)
    };
    match ask {
        PartyRequest::Create => {
            if party.is_none() {
                let id = settling.society.parties.create(by, settling.now);
                changed(settling, id);
            }
        }
        // An invitation is a request, and is asked as one.
        PartyRequest::Invite(_) => {}
        PartyRequest::Leave => {
            if let Some(party) = &party {
                let said = words.left(settling.name(by));
                tell(settling, party, by, &said);
                closed(settling, by, words.you_left());
                leave(settling, by);
            }
        }
        PartyRequest::Remove(number) => {
            if leads && let (Some(party), Some(removed)) = (&party, member(settling, number)) {
                let said = words.removed(settling.name(&removed));
                tell(settling, party, &removed, &said);
                settling.tell(&removed, feed(words.you_were_removed()));
                closed(settling, &removed, words.you_were_removed());
                leave(settling, &removed);
            }
        }
        PartyRequest::Lead(number) => {
            if leads && let (Some(party), Some(leader)) = (&party, member(settling, number)) {
                lead(settling, content, party.id, &leader);
            }
        }
        PartyRequest::Disband => {
            if leads && let Some(party) = &party {
                let said = words.disbanded(settling.name(by));
                for account in party.accounts() {
                    if account != by {
                        settling.tell(account, feed(&said));
                    }
                    closed(settling, account, &said);
                    settling.society.changed(account);
                    settling.society.parties.of.remove(account);
                    settling.society.parties.absent.remove(account);
                    social_request::end_of(settling, SocialRequestKind::PartyInvitation, account);
                }
                settling.society.parties.parties.remove(&party.id);
                settling
                    .unwritten
                    .remove::<CharacterParty>(record_id(party.id));
            }
        }
    }
}

/// Notes whose players are gone: a member gone for as long as the game's parties wait for one
/// leaves, and a party whose leader is gone is led by the member who has been in it longest of
/// those who are there.
pub(crate) fn keep(settling: &mut Settling, content: &Content) {
    let Some(rules) = &content.built::<GameSettings>().party else {
        return;
    };
    let now = settling.now;
    let members: Vec<AccountId> = settling.society.parties.of.keys().cloned().collect();
    let mut gone = Vec::new();
    for account in members {
        let here = settling.society.playing_in(&account).is_some();
        let absent = &mut settling.society.parties.absent;
        match (here, absent.get(&account).copied()) {
            (true, Some(_)) => {
                absent.remove(&account);
                each_member(settling, &account);
            }
            (false, None) => {
                absent.insert(account.clone(), now);
                each_member(settling, &account);
            }
            (false, Some(since)) if now.since(since) >= rules.absent_for => gone.push(account),
            _ => {}
        }
    }
    for account in gone {
        if let Some(party) = settling.society.parties.of_account(&account).cloned() {
            let said = messages::of(content).party.left(settling.name(&account));
            tell(settling, &party, &account, &said);
        }
        leave(settling, &account);
    }
    let leaderless: Vec<(PartyId, AccountId)> = settling
        .society
        .parties
        .parties
        .values()
        .filter(|party| settling.society.parties.absent.contains_key(&party.leader))
        .filter_map(|party| {
            let next = party
                .accounts()
                .find(|member| !settling.society.parties.absent.contains_key(*member))?;
            Some((party.id, next.clone()))
        })
        .collect();
    for (party, leader) in leaderless {
        lead(settling, content, party, &leader);
    }
}

/// Takes `account` out of the party it is in. A party it led is led by the member who has been
/// in it longest, and a party it was the last of is gone.
pub(crate) fn leave(settling: &mut Settling, account: &AccountId) {
    let parties = &mut settling.society.parties;
    let Some(id) = parties.of.remove(account) else {
        return;
    };
    parties.absent.remove(account);
    settling.society.changed(account);
    social_request::end_of(settling, SocialRequestKind::PartyInvitation, account);
    let parties = &mut settling.society.parties;
    let Some(party) = parties.parties.get_mut(&id) else {
        return;
    };
    party.members.retain(|member| member.account != *account);
    let Some(eldest) = party.members.first().map(|member| member.account.clone()) else {
        parties.parties.remove(&id);
        settling.unwritten.remove::<CharacterParty>(record_id(id));
        return;
    };
    if party.leader == *account {
        party.leader = eldest;
    }
    changed(settling, id);
}

/// Shows every member of the party `id` how it now stands, and has it saved.
fn changed(settling: &mut Settling, id: PartyId) {
    let Some(party) = settling.society.parties.parties.get(&id).cloned() else {
        return;
    };
    for account in party.accounts() {
        settling.society.changed(account);
    }
    settling.unwritten.put(party);
}

/// Shows every member of the party `account` is in how it now stands.
fn each_member(settling: &mut Settling, account: &AccountId) {
    let Some(party) = settling.society.parties.of_account(account) else {
        return;
    };
    let members: Vec<AccountId> = party.accounts().cloned().collect();
    for member in members {
        settling.society.changed(&member);
    }
}

fn lead(settling: &mut Settling, content: &Content, id: PartyId, leader: &AccountId) {
    let Some(party) = settling.society.parties.parties.get_mut(&id) else {
        return;
    };
    if party.leader == *leader || !party.holds(leader) {
        return;
    }
    party.leader = leader.clone();
    let party = party.clone();
    let said = messages::of(content).party.leads(settling.name(leader));
    for account in party.accounts() {
        settling.tell(account, feed(&said));
        settling.send(account, chat::notice(ChatChannel::Party, &said));
    }
    changed(settling, id);
}

/// Says what `by` typed to every member of the party they are in.
pub(crate) fn say(settling: &mut Settling, by: &AccountId, said: &ChatSaid) {
    let Some(party) = settling.society.parties.of_account(by).cloned() else {
        return;
    };
    let speaker = settling
        .reach(by)
        .and_then(|(world, character)| ChatSpeaker::of(world, character));
    for account in party.accounts() {
        if let Some(line) = chat::line_across(settling, ChatChannel::Party, by, account, said) {
            if let Some(speaker) = &speaker {
                settling.send(account, speaker.clone());
            }
            settling.send(account, line);
        }
    }
}

/// Tells every member of `party` but `but` what happened in it, the party's chat saying it too.
fn tell(settling: &mut Settling, party: &CharacterParty, but: &AccountId, said: &str) {
    for account in party.accounts().filter(|account| *account != but) {
        settling.tell(account, feed(said));
        settling.send(account, chat::notice(ChatChannel::Party, said));
    }
}

/// Has chat say why the party's channel is gone, to a player who is no longer in it.
fn closed(settling: &mut Settling, account: &AccountId, why: &str) {
    settling.send(account, chat::notice(ChatChannel::default(), why));
}

fn feed(said: &str) -> Notification {
    Notification::feed(HistoryTopic::Notification, said).into()
}

fn may_send(
    society: &Society,
    content: &Content,
    asker: &AccountId,
    asked: &AccountId,
) -> Result<(), RequestUnsent> {
    let Some(rules) = &content.built::<GameSettings>().party else {
        return Ok(());
    };
    let words = &messages::of(content).party;
    let name = |account: &AccountId| {
        society
            .accounts()
            .get(account)
            .map_or("", |asked| &asked.name)
    };
    if society.parties().of_account(asked).is_some() {
        return Err(RequestUnsent::Told(words.in_a_party(name(asked)).into()));
    }
    let Some(party) = society.parties().of_account(asker) else {
        return Ok(());
    };
    if party.members.len() >= rules.size as usize {
        return Err(RequestUnsent::Told(words.full().into()));
    }
    // There is only ever one invitation from a party to a character.
    let invited = party.accounts().any(|member| {
        society
            .requests()
            .between(SocialRequestKind::PartyInvitation, member, asked)
            .is_some()
    });
    if invited {
        Err(RequestUnsent::Quietly)
    } else {
        Ok(())
    }
}

fn accepted(
    settling: &mut Settling,
    content: &Content,
    request: &SocialRequest,
) -> Result<(), RequestUnaccepted> {
    let Some(rules) = &content.built::<GameSettings>().party else {
        return Ok(());
    };
    let words = &messages::of(content).party;
    let (inviter, joiner) = (&request.asker, &request.asked);
    if settling.society.parties.of_account(joiner).is_some() {
        let said = words.in_a_party(settling.name(joiner));
        return Err(RequestUnaccepted::Over(said.into()));
    }
    let id = match settling.society.parties.of.get(inviter) {
        Some(&id) => id,
        None => settling.society.parties.create(inviter, settling.now),
    };
    let parties = &mut settling.society.parties;
    let Some(party) = parties.parties.get_mut(&id) else {
        return Ok(());
    };
    if party.members.len() >= rules.size as usize {
        return Err(RequestUnaccepted::Over(words.full().into()));
    }
    party.members.push(PartyMember {
        account: joiner.clone(),
        joined: settling.now,
    });
    let party = party.clone();
    parties.of.insert(joiner.clone(), id);
    let said = words.joined(settling.name(joiner));
    tell(settling, &party, joiner, &said);
    let said = words.you_joined(settling.name(&party.leader));
    settling.send(joiner, chat::notice(ChatChannel::Party, &said));
    changed(settling, id);
    // A character is in one party at most: joining one denies the others.
    social_request::deny_others(settling, content, request);
    Ok(())
}

fn record_id(id: PartyId) -> RecordId {
    RecordId(id.0.to_string())
}

pub fn scripting(vocabulary: &mut Vocabulary) {
    query::scripting(vocabulary);
}
