//! Parties: a handful of players who chose to play together for now. A party is of the whole
//! server and not of a world, has one leader, and shares drops, experience and the kills its
//! quests count.

pub mod frames;
pub mod page;
pub mod share;

use std::collections::{BTreeMap, HashMap};

use bevy_app::App;
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::store::{Record, RecordId};
use crate::core::time::UnixMillis;
use crate::core::units::{Meters, Seconds};
use crate::systems::GameSettings;
use crate::systems::account::AccountId;
use crate::systems::area::AreaId;
use crate::systems::chat::{self, ChatChannel, ChatChannelSource};
use crate::systems::history::HistoryTopic;
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
pub struct PlayerParty {
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

impl Record for PlayerParty {
    const KIND: &'static str = "party";

    fn id(&self) -> RecordId {
        record_id(self.id)
    }
}

impl PlayerParty {
    pub fn holds(&self, account: &AccountId) -> bool {
        self.members.iter().any(|member| member.account == *account)
    }

    pub fn accounts(&self) -> impl Iterator<Item = &AccountId> {
        self.members.iter().map(|member| &member.account)
    }
}

/// Every party of the server.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlayerParties {
    parties: BTreeMap<PartyId, PlayerParty>,
    of: HashMap<AccountId, PartyId>,
    /// Since when each member's player has been gone.
    absent: HashMap<AccountId, UnixMillis>,
    dealt: u64,
}

impl PlayerParties {
    pub fn of(saved: impl IntoIterator<Item = PlayerParty>) -> PlayerParties {
        let mut parties = PlayerParties::default();
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
    pub fn of_account(&self, account: &AccountId) -> Option<&PlayerParty> {
        self.parties.get(self.of.get(account)?)
    }

    /// Whether the two are in one party.
    pub fn together(&self, one: &AccountId, other: &AccountId) -> bool {
        self.of
            .get(one)
            .is_some_and(|party| self.of.get(other) == Some(party))
    }

    /// Since when the player of `account`, a member of a party, has been gone.
    pub fn absent_since(&self, account: &AccountId) -> Option<UnixMillis> {
        self.absent.get(account).copied()
    }

    fn create(&mut self, leader: &AccountId, now: UnixMillis) -> PartyId {
        self.dealt += 1;
        let id = PartyId(self.dealt);
        let party = PlayerParty {
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
    pub player: SocietyMember,
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
                        player: society.member(member)?,
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
        send: |world, speaker, text| {
            if let Some(client) = world.get::<PlayedBy>(speaker).map(|by| by.client) {
                society::ask(world, client, SocietyAsk::PartyLine(text));
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
        words: |words| SocialRequestWords {
            asks: &words.party_invite_asks,
            asked: &words.party_invite_asked,
            sent: None,
            accepted: None,
            declined: Some(&words.party_invite_declined),
            lapsed: Some(&words.party_invite_lapsed),
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

/// Does what `by` asked of parties, under the game `settings` are of.
pub(crate) fn settle(
    settling: &mut Settling,
    settings: &GameSettings,
    by: &AccountId,
    ask: PartyRequest,
) {
    if settings.party.is_none() {
        return;
    }
    let words = &settings.words;
    let party = settling.society.parties.of_account(by).cloned();
    let leads = party.as_ref().is_some_and(|party| party.leader == *by);
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
                let said = words.party_left.replace("{name}", &settling.name(by));
                tell(settling, party, by, &said);
                closed(settling, by, &words.you_left_party);
                leave(settling, by);
            }
        }
        PartyRequest::Remove(number) => {
            if leads && let (Some(party), Some(removed)) = (&party, member(settling, number)) {
                let said = words
                    .party_removed
                    .replace("{name}", &settling.name(&removed));
                tell(settling, party, &removed, &said);
                settling.tell(&removed, feed(&words.you_were_removed));
                closed(settling, &removed, &words.you_were_removed);
                leave(settling, &removed);
            }
        }
        PartyRequest::Lead(number) => {
            if leads && let (Some(party), Some(leader)) = (&party, member(settling, number)) {
                lead(settling, settings, party.id, &leader);
            }
        }
        PartyRequest::Disband => {
            if leads && let Some(party) = &party {
                let said = words.party_disbanded.replace("{name}", &settling.name(by));
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
                    .remove::<PlayerParty>(record_id(party.id));
            }
        }
    }
}

/// Notes whose players are gone: a member gone for as long as the game's parties wait for one
/// leaves, and a party whose leader is gone is led by the member who has been in it longest of
/// those who are there.
pub(crate) fn keep(settling: &mut Settling, settings: &GameSettings) {
    let Some(rules) = &settings.party else {
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
            let said = settings
                .words
                .party_left
                .replace("{name}", &settling.name(&account));
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
        lead(settling, settings, party, &leader);
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
        settling.unwritten.remove::<PlayerParty>(record_id(id));
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

fn lead(settling: &mut Settling, settings: &GameSettings, id: PartyId, leader: &AccountId) {
    let Some(party) = settling.society.parties.parties.get_mut(&id) else {
        return;
    };
    if party.leader == *leader || !party.holds(leader) {
        return;
    }
    party.leader = leader.clone();
    let party = party.clone();
    let said = settings
        .words
        .party_leads
        .replace("{name}", &settling.name(leader));
    for account in party.accounts() {
        settling.tell(account, feed(&said));
        settling.send(account, chat::notice(ChatChannel::Party, &said));
    }
    changed(settling, id);
}

/// Says what `by` typed to every member of the party they are in.
pub(crate) fn say(settling: &mut Settling, by: &AccountId, text: &str) {
    let Some(party) = settling.society.parties.of_account(by).cloned() else {
        return;
    };
    for account in party.accounts() {
        if let Some(line) = chat::line_across(settling, ChatChannel::Party, by, account, text) {
            settling.send(account, line);
        }
    }
}

/// Tells every member of `party` but `but` what happened in it, the party's chat saying it too.
fn tell(settling: &mut Settling, party: &PlayerParty, but: &AccountId, said: &str) {
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
    settings: &GameSettings,
    asker: &AccountId,
    asked: &AccountId,
) -> Result<(), RequestUnsent> {
    let Some(rules) = &settings.party else {
        return Ok(());
    };
    let words = &settings.words;
    let name = |account: &AccountId| {
        society
            .accounts()
            .get(account)
            .map_or("", |asked| &asked.name)
    };
    if society.parties().of_account(asked).is_some() {
        return Err(RequestUnsent::Told(
            words.in_a_party.replace("{name}", name(asked)),
        ));
    }
    let Some(party) = society.parties().of_account(asker) else {
        return Ok(());
    };
    if party.members.len() >= rules.size as usize {
        return Err(RequestUnsent::Told(words.party_full.clone()));
    }
    // There is only ever one invitation from a party to a player.
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
    settings: &GameSettings,
    request: &SocialRequest,
) -> Result<(), RequestUnaccepted> {
    let Some(rules) = &settings.party else {
        return Ok(());
    };
    let words = &settings.words;
    let (inviter, joiner) = (&request.asker, &request.asked);
    if settling.society.parties.of_account(joiner).is_some() {
        let said = words.in_a_party.replace("{name}", &settling.name(joiner));
        return Err(RequestUnaccepted::Over(said));
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
        return Err(RequestUnaccepted::Over(words.party_full.clone()));
    }
    party.members.push(PartyMember {
        account: joiner.clone(),
        joined: settling.now,
    });
    let party = party.clone();
    parties.of.insert(joiner.clone(), id);
    let said = words.party_joined.replace("{name}", &settling.name(joiner));
    tell(settling, &party, joiner, &said);
    let said = words
        .you_joined_party
        .replace("{name}", &settling.name(&party.leader));
    settling.send(joiner, chat::notice(ChatChannel::Party, &said));
    changed(settling, id);
    // A player is in one party at most: joining one declines the others.
    social_request::decline_others(settling, settings, request);
    Ok(())
}

fn record_id(id: PartyId) -> RecordId {
    RecordId(id.0.to_string())
}
