//! A request between two players: one asks, the other says yes or no. Friendship, an invitation
//! and a seat are all requests, each a kind the system that owns it brings, and every one of
//! them passes the same gate on its way out.

pub mod block;
pub mod card;
pub(crate) mod setting;

use std::collections::{BTreeMap, BTreeSet, HashMap};

use bevy_app::App;
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{IntoStaticStr, VariantArray};

pub use block::{Block, BlockAsk, BlockLists};

use crate::core::store::{Record, RecordId};
use crate::core::time::UnixMillis;
use crate::core::units::Seconds;
use crate::systems::account::AccountId;
use crate::systems::history::HistoryTopic;
use crate::systems::notification::Notification;
use crate::systems::player::roster::ServerRoster;
use crate::systems::player::{self, PlayedBy};
use crate::systems::society::{self, AccountNumber, Settling, Society, SocietyAsk};
use crate::systems::{GameSettings, GameWords};

pub fn register(app: &mut App) {
    use bevy_replication::AppMessageExt;
    app.add_client_message::<AnswerSocialRequest>()
        .add_client_message::<BlockPlayer>()
        .add_client_message::<ChooseAcceptsRequests>();
}

/// The kinds of request there are. Each is brought by the system that owns it, and requests
/// know nothing else of what it asks for.
#[derive(
    JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(rename_all = "snake_case")]
pub enum SocialRequestKind {
    Friendship,
    PartyInvitation,
    Seat,
}

/// Who an account takes requests from. The server keeps it with the account and holds every
/// request to it, of every kind.
#[derive(
    JsonSchema,
    Serialize,
    Deserialize,
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    IntoStaticStr,
    VariantArray,
)]
#[serde(rename_all = "snake_case")]
pub enum AcceptsRequestsFrom {
    #[default]
    Everyone,
    Friends,
    Party,
    Nobody,
}

/// How a game has requests between its players work.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestSettings {
    /// How long a request's card stays on the HUD of the player asked.
    pub card_shows: Seconds,
    /// Who an account takes requests from until its player chooses.
    pub new_accounts_accept: AcceptsRequestsFrom,
}

/// What a system brings for its kind of request.
#[derive(Clone, Copy)]
pub struct SocialRequestSource {
    pub kind: SocialRequestKind,
    /// How long it waits for its answer, in a game that has the kind.
    pub lasts: fn(&GameSettings) -> Option<Seconds>,
    /// Whether it reaches a player who is not playing, and so is saved until they are.
    pub reaches_offline: bool,
    pub words: fn(&GameWords) -> SocialRequestWords<'_>,
    /// Whether the first account may ask the second, by the kind's own rules, once the gate
    /// every request passes has let it through.
    pub may_send: fn(&Society, &GameSettings, &AccountId, &AccountId) -> Result<(), RequestUnsent>,
    /// What a yes does.
    pub accepted: fn(&mut Settling, &GameSettings, &SocialRequest) -> Result<(), RequestUnaccepted>,
}

/// Why a request was not sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestUnsent {
    /// The asker is told nothing.
    Quietly,
    /// The asker is told why, in the game's words.
    Told(String),
}

/// Why a yes did not do what its kind does, in the game's words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestUnaccepted {
    /// The request waits on, and the player who said yes is told why.
    Waits(String),
    /// The request is over, and both are told why.
    Over(String),
}

/// What a kind of request says, in the game's words. `{name}` is the other player and `{about}`
/// what the request is about, where its kind names something.
#[derive(Clone, Copy, Debug)]
pub struct SocialRequestWords<'a> {
    /// What the player asked reads on its card.
    pub asks: &'a str,
    /// What the asker reads while it waits.
    pub asked: &'a str,
    /// What the asker is told as it is sent, for a kind that tells them.
    pub sent: Option<&'a str>,
    /// What the asker is told on a yes, for a kind that tells them.
    pub accepted: Option<&'a str>,
    /// What the asker is told on a no. A kind that does not tell them keeps a request that was
    /// declined waiting as far as its asker sees, until its time is up.
    pub declined: Option<&'a str>,
    /// What the asker is told as its time is up unanswered, for a kind that tells them.
    pub lapsed: Option<&'a str>,
}

#[derive(
    JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
pub struct SocialRequestId(pub u64);

/// One player asking another for something, waiting for its answer.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SocialRequest {
    pub id: SocialRequestId,
    pub kind: SocialRequestKind,
    pub asker: AccountId,
    pub asked: AccountId,
    /// What it is about, for a kind whose words name something.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    /// Until when its card shows on the HUD.
    pub shown_until: UnixMillis,
    /// Until when it waits.
    pub until: UnixMillis,
    /// Whether the player asked said no, of a kind that does not tell the asker.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub declined: bool,
}

impl Record for SocialRequest {
    const KIND: &'static str = "request";

    fn id(&self) -> RecordId {
        RecordId(self.id.0.to_string())
    }
}

/// Every request waiting for its answer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SocialRequests {
    waiting: BTreeMap<SocialRequestId, SocialRequest>,
    of_account: HashMap<AccountId, BTreeSet<SocialRequestId>>,
    ends: BTreeSet<(UnixMillis, SocialRequestId)>,
    dealt: u64,
}

impl SocialRequests {
    pub fn of(saved: impl IntoIterator<Item = SocialRequest>) -> SocialRequests {
        let mut requests = SocialRequests::default();
        for request in saved {
            requests.dealt = requests.dealt.max(request.id.0);
            requests.hold(request);
        }
        requests
    }

    pub fn get(&self, id: SocialRequestId) -> Option<&SocialRequest> {
        self.waiting.get(&id)
    }

    /// The requests `account` asked or is asked, the oldest first.
    pub fn of_account(&self, account: &AccountId) -> impl Iterator<Item = &SocialRequest> {
        self.of_account
            .get(account)
            .into_iter()
            .flatten()
            .filter_map(|id| self.waiting.get(id))
    }

    /// The request of `kind` that `asker` has waiting with `asked`.
    pub fn between(
        &self,
        kind: SocialRequestKind,
        asker: &AccountId,
        asked: &AccountId,
    ) -> Option<&SocialRequest> {
        self.of_account(asker).find(|request| {
            request.kind == kind && request.asker == *asker && request.asked == *asked
        })
    }

    fn add(&mut self, request: SocialRequest) -> SocialRequest {
        self.dealt += 1;
        let request = SocialRequest {
            id: SocialRequestId(self.dealt),
            ..request
        };
        self.hold(request.clone());
        request
    }

    fn hold(&mut self, request: SocialRequest) {
        for account in [&request.asker, &request.asked] {
            self.of_account
                .entry(account.clone())
                .or_default()
                .insert(request.id);
        }
        self.ends.insert((request.until, request.id));
        self.waiting.insert(request.id, request);
    }

    /// Notes that the player asked said no to a request that waits on: the request as it then
    /// is.
    fn decline(&mut self, id: SocialRequestId) -> Option<SocialRequest> {
        let request = self.waiting.get_mut(&id)?;
        request.declined = true;
        Some(request.clone())
    }

    fn remove(&mut self, id: SocialRequestId) -> Option<SocialRequest> {
        let request = self.waiting.remove(&id)?;
        for account in [&request.asker, &request.asked] {
            if let Some(waiting) = self.of_account.get_mut(account) {
                waiting.remove(&id);
                if waiting.is_empty() {
                    self.of_account.remove(account);
                }
            }
        }
        self.ends.remove(&(request.until, id));
        Some(request)
    }

    /// The first request whose wait is over at `now`.
    fn ended(&self, now: UnixMillis) -> Option<SocialRequestId> {
        self.ends
            .first()
            .filter(|(until, _)| *until <= now)
            .map(|&(_, id)| id)
    }
}

/// Why a request is not sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestRefusal {
    /// The player asked keeps the asker away, or takes no requests from them. The two read the
    /// same, so nobody learns that they are blocked by asking.
    NotTaking,
    /// The asker keeps the player asked away.
    YouBlocked,
    /// The request needs the player asked at their character, and they are not.
    NotHere,
}

impl RequestRefusal {
    /// What the asker is told, of the player called `name`.
    pub fn describe(self, words: &GameWords, name: &str) -> String {
        let says = match self {
            RequestRefusal::NotTaking => &words.not_taking_requests,
            RequestRefusal::YouBlocked => &words.you_blocked,
            RequestRefusal::NotHere => &words.not_here,
        };
        says.replace("{name}", name)
    }
}

/// The one gate every request passes: whether `asker` may send `asked` a request of the kind
/// `source` brings.
pub fn may_ask(
    society: &Society,
    roster: &ServerRoster,
    settings: &RequestSettings,
    source: &SocialRequestSource,
    asker: &AccountId,
    asked: &AccountId,
) -> Result<(), RequestRefusal> {
    if society.blocks().blocks(asker, asked) {
        return Err(RequestRefusal::YouBlocked);
    }
    if society.blocks().blocks(asked, asker) || !society.admits(asked, asker, settings) {
        return Err(RequestRefusal::NotTaking);
    }
    let here = roster.account(asked).is_some_and(|enrolled| !enrolled.away);
    if !source.reaches_offline && !here {
        return Err(RequestRefusal::NotHere);
    }
    Ok(())
}

/// What a player asks be done with a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SocialRequestAsk {
    Ask {
        kind: SocialRequestKind,
        of: AccountNumber,
        about: Option<String>,
    },
    Accept(SocialRequestId),
    Decline(SocialRequestId),
    TakeBack(SocialRequestId),
}

/// A player's answer to a request on their card: a yes or a no from the one asked, or the asker
/// taking it back.
#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnswerSocialRequest {
    pub request: SocialRequestId,
    pub answer: RequestAnswer,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestAnswer {
    Accept,
    Decline,
    TakeBack,
}

/// A player blocking another, or undoing it.
#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockPlayer {
    pub account: AccountNumber,
    pub blocked: bool,
}

/// A player choosing who they take requests from.
#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChooseAcceptsRequests(pub AcceptsRequestsFrom);

/// Hands what players of this world asked of requests, blocks and their setting to society.
pub fn requests(world: &mut World) {
    let client = |world: &World, player: Entity| world.get::<PlayedBy>(player).map(|by| by.client);
    player::player_requests::<AnswerSocialRequest>(world, false, |world, player, answer| {
        let ask = match answer.answer {
            RequestAnswer::Accept => SocialRequestAsk::Accept(answer.request),
            RequestAnswer::Decline => SocialRequestAsk::Decline(answer.request),
            RequestAnswer::TakeBack => SocialRequestAsk::TakeBack(answer.request),
        };
        if let Some(client) = client(world, player) {
            society::ask(world, client, SocietyAsk::Request(ask));
        }
        Ok(())
    });
    player::player_requests::<BlockPlayer>(world, false, |world, player, block| {
        let ask = if block.blocked {
            BlockAsk::Block(block.account)
        } else {
            BlockAsk::Unblock(block.account)
        };
        if let Some(client) = client(world, player) {
            society::ask(world, client, SocietyAsk::Block(ask));
        }
        Ok(())
    });
    player::player_requests::<ChooseAcceptsRequests>(world, false, |world, player, chosen| {
        if let Some(client) = client(world, player) {
            society::ask(world, client, SocietyAsk::Accepts(chosen.0));
        }
        Ok(())
    });
}

/// Does with a request what `by` asked, under the game `settings` are of.
pub(crate) fn settle(
    settling: &mut Settling,
    settings: &GameSettings,
    by: &AccountId,
    ask: SocialRequestAsk,
) {
    match ask {
        SocialRequestAsk::Ask { kind, of, about } => {
            let Some(asked) = settling.society.accounts().numbered(of).cloned() else {
                return;
            };
            send(settling, settings, kind, by, &asked, about);
        }
        SocialRequestAsk::Accept(id) => {
            if waits_for(settling, id, |request| {
                request.asked == *by && !request.declined
            }) {
                accept(settling, settings, id);
            }
        }
        SocialRequestAsk::Decline(id) => {
            if waits_for(settling, id, |request| {
                request.asked == *by && !request.declined
            }) {
                decline(settling, settings, id);
            }
        }
        SocialRequestAsk::TakeBack(id) => {
            if waits_for(settling, id, |request| request.asker == *by) {
                end(settling, id);
            }
        }
    }
}

/// Ends every request whose wait is over, under the game `settings` are of.
pub(crate) fn lapse(settling: &mut Settling, settings: &GameSettings) {
    while let Some(id) = settling.society.requests.ended(settling.now) {
        let Some(request) = end(settling, id) else {
            continue;
        };
        let lapsed = words_of(settling, settings, request.kind).and_then(|says| says.lapsed);
        if let Some(lapsed) = lapsed.filter(|_| !request.declined) {
            let text = lapsed.replace("{name}", &settling.name(&request.asked));
            let told = Notification::feed(HistoryTopic::Notification, text).failed();
            settling.tell(&request.asker, told);
        }
    }
}

/// Ends every request of `kind` that `asker` has waiting, with nobody told.
pub(crate) fn end_of(settling: &mut Settling, kind: SocialRequestKind, asker: &AccountId) {
    let asked: Vec<SocialRequestId> = settling
        .society
        .requests
        .of_account(asker)
        .filter(|request| request.kind == kind && request.asker == *asker)
        .map(|request| request.id)
        .collect();
    for id in asked {
        end(settling, id);
    }
}

/// Declines every other request of its kind that the player `accepted` was asked of has
/// waiting, as a yes to one is a no to the rest.
pub(crate) fn decline_others(
    settling: &mut Settling,
    settings: &GameSettings,
    accepted: &SocialRequest,
) {
    let others: Vec<SocialRequestId> = settling
        .society
        .requests
        .of_account(&accepted.asked)
        .filter(|request| {
            request.kind == accepted.kind
                && request.asked == accepted.asked
                && request.id != accepted.id
        })
        .map(|request| request.id)
        .collect();
    for id in others {
        decline(settling, settings, id);
    }
}

/// Ends every request waiting between the two, whoever asked.
fn end_between(settling: &mut Settling, one: &AccountId, other: &AccountId) {
    let between: Vec<SocialRequestId> = settling
        .society
        .requests
        .of_account(one)
        .filter(|request| request.asker == *other || request.asked == *other)
        .map(|request| request.id)
        .collect();
    for id in between {
        end(settling, id);
    }
}

fn send(
    settling: &mut Settling,
    settings: &GameSettings,
    kind: SocialRequestKind,
    asker: &AccountId,
    asked: &AccountId,
    about: Option<String>,
) {
    if asker == asked {
        return;
    }
    let Some(source) = settling.society.source(kind) else {
        return;
    };
    let Some(lasts) = (source.lasts)(settings) else {
        return;
    };
    let requests = &settling.society.requests;
    if requests.between(kind, asker, asked).is_some() {
        return;
    }
    // Two who ask each other for the same thing have both said yes.
    if let Some(answered) = requests
        .between(kind, asked, asker)
        .map(|request| request.id)
    {
        accept(settling, settings, answered);
        return;
    }
    let gate = may_ask(
        settling.society,
        settling.roster,
        &settings.requests,
        &source,
        asker,
        asked,
    );
    let gate = gate
        .map_err(|refusal| {
            RequestUnsent::Told(refusal.describe(&settings.words, &settling.name(asked)))
        })
        .and_then(|()| (source.may_send)(settling.society, settings, asker, asked));
    if let Err(unsent) = gate {
        if let RequestUnsent::Told(why) = unsent {
            settling.refuse(asker, why);
        }
        return;
    }
    let until = settling.now.after(lasts);
    let shown_until = settling.now.after(settings.requests.card_shows).min(until);
    let request = settling.society.requests.add(SocialRequest {
        id: SocialRequestId(0),
        kind,
        asker: asker.clone(),
        asked: asked.clone(),
        about,
        shown_until,
        until,
        declined: false,
    });
    settling.society.changed(asker);
    settling.society.changed(asked);
    if source.reaches_offline {
        settling.unwritten.put(request);
    }
    if let Some(sent) = (source.words)(&settings.words).sent {
        let text = sent.replace("{name}", &settling.name(asked));
        settling.tell(asker, Notification::feed(HistoryTopic::Notification, text));
    }
}

fn accept(settling: &mut Settling, settings: &GameSettings, id: SocialRequestId) {
    let Some(request) = settling.society.requests.get(id).cloned() else {
        return;
    };
    let Some(source) = settling.society.source(request.kind) else {
        return;
    };
    match (source.accepted)(settling, settings, &request) {
        Ok(()) => {
            end(settling, id);
            if let Some(accepted) = (source.words)(&settings.words).accepted {
                let text = accepted.replace("{name}", &settling.name(&request.asked));
                let told = Notification::feed(HistoryTopic::Notification, text);
                settling.tell(&request.asker, told);
            }
        }
        Err(RequestUnaccepted::Waits(why)) => settling.refuse(&request.asked, why),
        Err(RequestUnaccepted::Over(why)) => {
            end(settling, id);
            settling.refuse(&request.asker, why.clone());
            settling.refuse(&request.asked, why);
        }
    }
}

fn decline(settling: &mut Settling, settings: &GameSettings, id: SocialRequestId) {
    let Some(said) = settling
        .society
        .requests
        .get(id)
        .map(|request| request.kind)
    else {
        return;
    };
    let Some(declined) = words_of(settling, settings, said).and_then(|says| says.declined) else {
        if let Some(request) = settling.society.requests.decline(id) {
            settling.society.changed(&request.asked);
            let saved = settling.society.source(request.kind);
            if saved.is_some_and(|source| source.reaches_offline) {
                settling.unwritten.put(request);
            }
        }
        return;
    };
    let Some(request) = end(settling, id) else {
        return;
    };
    let text = declined.replace("{name}", &settling.name(&request.asked));
    let told = Notification::feed(HistoryTopic::Notification, text).failed();
    settling.tell(&request.asker, told);
}

/// Takes the request out of those waiting, for both its players and for the store.
fn end(settling: &mut Settling, id: SocialRequestId) -> Option<SocialRequest> {
    let request = settling.society.requests.remove(id)?;
    settling.society.changed(&request.asker);
    settling.society.changed(&request.asked);
    let saved = settling
        .society
        .source(request.kind)
        .is_some_and(|source| source.reaches_offline);
    if saved {
        settling.unwritten.remove::<SocialRequest>(request.id());
    }
    Some(request)
}

fn waits_for(
    settling: &Settling,
    id: SocialRequestId,
    is: impl FnOnce(&SocialRequest) -> bool,
) -> bool {
    settling.society.requests.get(id).is_some_and(is)
}

fn words_of<'a>(
    settling: &Settling,
    settings: &'a GameSettings,
    kind: SocialRequestKind,
) -> Option<SocialRequestWords<'a>> {
    let source = settling.society.source(kind)?;
    Some((source.words)(&settings.words))
}
