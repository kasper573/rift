//! Friends: the players an account has agreed to know. A friendship is between two accounts,
//! always goes both ways, begins with a request one of them accepts and ends when either says
//! so.

pub mod page;
pub mod search;

use std::collections::{BTreeMap, HashMap};

use bevy_app::App;
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use search::{CharactersFound, FindCharacters};

use crate::core::store::{Record, RecordId};
use crate::core::time::UnixMillis;
use crate::core::units::Seconds;
use crate::systems::GameSettings;
use crate::systems::account::AccountId;
use crate::systems::player::RequestNeeds;
use crate::systems::player::{self, PlayedBy};
use crate::systems::social_request::{
    RequestUnaccepted, RequestUnsent, SocialRequest, SocialRequestAsk, SocialRequestKind,
    SocialRequestSource, SocialRequestWords,
};
use crate::systems::society::{self, AccountNumber, Settling, Society, SocietyAsk};

pub fn register(app: &mut App) {
    use bevy_replication::AppMessageExt;
    app.add_client_message::<AskFriendship>()
        .add_client_message::<RemoveFriend>()
        .add_client_message::<FindCharacters>()
        .add_server_message::<CharactersFound>();
}

/// How a game has friends work. A game without it has no friends.
#[derive(JsonSchema, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FriendSettings {
    /// How many friends an account's list holds, counting those it has asked and waits for.
    pub most: u32,
    /// How many requests for friendship an account may have waiting for their answer.
    pub requests: u32,
    /// How long a request for friendship waits for its answer.
    pub request_lasts: Seconds,
}

/// That two accounts are friends, since when. `one` is the lesser id of the two.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Friendship {
    pub one: AccountId,
    pub other: AccountId,
    pub since: UnixMillis,
}

impl Record for Friendship {
    const KIND: &'static str = "friendship";

    fn id(&self) -> RecordId {
        Friendship::id_of(&self.one, &self.other)
    }
}

impl Friendship {
    fn between(a: &AccountId, b: &AccountId, since: UnixMillis) -> Friendship {
        let (one, other) = if a <= b { (a, b) } else { (b, a) };
        Friendship {
            one: one.clone(),
            other: other.clone(),
            since,
        }
    }

    /// The lesser id is led by its length, so no two pairs of ids make one record id.
    fn id_of(a: &AccountId, b: &AccountId) -> RecordId {
        let (one, other) = if a <= b { (a, b) } else { (b, a) };
        RecordId(format!("{}:{}{}", one.0.len(), one.0, other.0))
    }
}

/// Who every account is friends with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Friendships(HashMap<AccountId, BTreeMap<AccountId, UnixMillis>>);

impl Friendships {
    pub fn of(saved: impl IntoIterator<Item = Friendship>) -> Friendships {
        let mut friendships = Friendships::default();
        for friendship in saved {
            friendships.join(&friendship.one, &friendship.other, friendship.since);
        }
        friendships
    }

    pub fn are(&self, one: &AccountId, other: &AccountId) -> bool {
        self.0
            .get(one)
            .is_some_and(|friends| friends.contains_key(other))
    }

    /// The friends of `account`.
    pub fn of_account(&self, account: &AccountId) -> impl Iterator<Item = &AccountId> {
        self.0.get(account).into_iter().flat_map(BTreeMap::keys)
    }

    pub fn count(&self, account: &AccountId) -> usize {
        self.0.get(account).map_or(0, BTreeMap::len)
    }

    fn join(&mut self, one: &AccountId, other: &AccountId, since: UnixMillis) -> bool {
        if self.are(one, other) {
            return false;
        }
        for (of, friend) in [(one, other), (other, one)] {
            self.0
                .entry(of.clone())
                .or_default()
                .insert(friend.clone(), since);
        }
        true
    }

    fn part(&mut self, one: &AccountId, other: &AccountId) -> bool {
        let mut parted = false;
        for (of, friend) in [(one, other), (other, one)] {
            let Some(friends) = self.0.get_mut(of) else {
                continue;
            };
            parted |= friends.remove(friend).is_some();
            if friends.is_empty() {
                self.0.remove(of);
            }
        }
        parted
    }
}

/// What a player asks of friends beyond a request: an end to a friendship, or players by the
/// start of their name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FriendAsk {
    Remove(AccountNumber),
    Find(String),
}

/// A player asking another to be their friend.
#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct AskFriendship(pub AccountNumber);

/// A player ending a friendship.
#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct RemoveFriend(pub AccountNumber);

/// Friendship as a kind of request: it reaches a player who is gone and waits for them, and a
/// no is not told.
pub fn request_source() -> SocialRequestSource {
    SocialRequestSource {
        kind: SocialRequestKind::Friendship,
        lasts: |settings| settings.friends.map(|friends| friends.request_lasts),
        reaches_offline: true,
        words: |words| SocialRequestWords {
            asks: &words.friendship_asks,
            asked: &words.friendship_asked,
            sent: &words.friendship_sent,
            accepted: Some(&words.friendship_accepted),
            denied: None,
            lapsed: None,
        },
        may_send,
        accepted,
        stands: |_, _| true,
    }
}

/// Hands what players of this world asked of friends to society.
pub fn requests(world: &mut World) {
    let client = |world: &World, player: Entity| world.get::<PlayedBy>(player).map(|by| by.client);
    player::player_requests::<AskFriendship>(world, RequestNeeds::Nothing, |world, player, ask| {
        let ask = SocialRequestAsk::Ask {
            kind: SocialRequestKind::Friendship,
            of: ask.0,
            about: None,
        };
        if let Some(client) = client(world, player) {
            society::ask(world, client, SocietyAsk::Request(ask));
        }
        Ok(())
    });
    player::player_requests::<RemoveFriend>(
        world,
        RequestNeeds::Nothing,
        |world, player, remove| {
            if let Some(client) = client(world, player) {
                society::ask(
                    world,
                    client,
                    SocietyAsk::Friend(FriendAsk::Remove(remove.0)),
                );
            }
            Ok(())
        },
    );
    player::player_requests::<FindCharacters>(
        world,
        RequestNeeds::Nothing,
        |world, player, find| {
            if let Some(client) = client(world, player) {
                society::ask(
                    world,
                    client,
                    SocietyAsk::Friend(FriendAsk::Find(find.0.clone())),
                );
            }
            Ok(())
        },
    );
}

/// Does what `by` asked of friends, under the game `settings` are of.
pub(crate) fn settle(
    settling: &mut Settling,
    settings: &GameSettings,
    by: &AccountId,
    ask: FriendAsk,
) {
    match ask {
        FriendAsk::Remove(number) => {
            if let Some(friend) = settling.society.accounts().numbered(number).cloned() {
                part(settling, by, &friend);
            }
        }
        FriendAsk::Find(starts) => {
            let found = search::find(settling.society, &starts, settings.suggestions as usize);
            settling.send(by, found);
        }
    }
}

/// Ends the friendship of the two, when there is one. Neither is told.
pub(crate) fn part(settling: &mut Settling, one: &AccountId, other: &AccountId) {
    if !settling.society.friends.part(one, other) {
        return;
    }
    settling
        .unwritten
        .remove::<Friendship>(Friendship::id_of(one, other));
    settling.society.changed(one);
    settling.society.changed(other);
}

/// How many more friends the list of `account` has room for, counting those it has asked and
/// waits for as friends already, so accepting always finds the room it was asked with.
fn room(society: &Society, settings: &FriendSettings, account: &AccountId) -> usize {
    let held = society.friends().count(account) + sent(society, account);
    (settings.most as usize).saturating_sub(held)
}

/// How many requests for friendship `account` has waiting for their answer.
fn sent(society: &Society, account: &AccountId) -> usize {
    society
        .requests()
        .of_account(account)
        .filter(|request| {
            request.kind == SocialRequestKind::Friendship && request.asker == *account
        })
        .count()
}

fn may_send(
    society: &Society,
    settings: &GameSettings,
    asker: &AccountId,
    asked: &AccountId,
) -> Result<(), RequestUnsent> {
    let Some(friends) = &settings.friends else {
        return Err(RequestUnsent::Quietly);
    };
    let words = &settings.words;
    if society.friends().are(asker, asked) {
        let name = society
            .accounts()
            .get(asked)
            .map_or("", |asked| &asked.name);
        return Err(RequestUnsent::Told(
            words.already_friends.replace("{name}", name),
        ));
    }
    if room(society, friends, asker) == 0 {
        return Err(RequestUnsent::Told(words.friends_full.clone()));
    }
    if sent(society, asker) >= friends.requests as usize {
        return Err(RequestUnsent::Told(words.too_many_friend_requests.clone()));
    }
    Ok(())
}

fn accepted(
    settling: &mut Settling,
    settings: &GameSettings,
    request: &SocialRequest,
) -> Result<(), RequestUnaccepted> {
    let full = settings
        .friends
        .is_some_and(|friends| room(settling.society, &friends, &request.asked) == 0);
    if full {
        return Err(RequestUnaccepted::Waits(
            settings.words.friends_full.clone(),
        ));
    }
    let (one, other) = (&request.asker, &request.asked);
    if settling.society.friends.join(one, other, settling.now) {
        settling
            .unwritten
            .put(Friendship::between(one, other, settling.now));
    }
    Ok(())
}
