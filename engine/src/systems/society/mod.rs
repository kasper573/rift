//! Everything of the server and of no one world: who has signed in, and what players share.
//! There is one of it, changed only where the server's worlds confer.

pub mod account;
pub mod tab;
pub mod view;

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

pub use account::{Account, AccountNumber, Accounts};
use bevy_ecs::prelude::*;
pub use view::{FriendRow, RequestCard, SocietyMember, SocietyView};

use crate::core::content::{Content, ContentScope};
use crate::core::store::{self, Record, RecordStore, RecordStoreProblem};
use crate::core::time::UnixMillis;
use crate::systems::GameSettings;
use crate::systems::account::{AccountId, AccountIdentity};
use crate::systems::area::AreaId;
use crate::systems::friend::{self, FriendAsk, Friendship, Friendships};
use crate::systems::inbox::{self, InboxAsk, InboxMessage, InboxReads, Inboxes};
use crate::systems::notification::{self, Notification};
use crate::systems::party::{self, PartyRequest, PlayerParties, PlayerParty};
use crate::systems::player::roster::ServerRoster;
use crate::systems::player::{ClientId, Connected, PlayedBy, Players};
use crate::systems::save::Unwritten;
use crate::systems::social_request::{
    self, AcceptsRequestsFrom, Block, BlockAsk, BlockLists, RequestSettings, SocialRequest,
    SocialRequestAsk, SocialRequestKind, SocialRequestSource, SocialRequests,
};
use crate::systems::visibility;
use crate::systems::worlds::AboveWorlds;

/// Where an account's character stands, and whether its player is away from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Playing {
    area: AreaId,
    away: bool,
}

#[derive(Default)]
pub struct Society {
    accounts: Accounts,
    pub(crate) friends: Friendships,
    pub(crate) blocks: BlockLists,
    pub(crate) requests: SocialRequests,
    pub(crate) inboxes: Inboxes,
    pub(crate) parties: PlayerParties,
    sources: Vec<SocialRequestSource>,
    /// The accounts whose characters stand in a world, and where.
    playing: HashMap<AccountId, Playing>,
    /// The accounts whose players see something of society that changed since they were shown.
    changed: BTreeSet<AccountId>,
}

impl Society {
    /// A society that holds nothing yet, where requests are of the kinds `sources` bring.
    pub fn new(sources: Vec<SocialRequestSource>) -> Society {
        Society {
            sources,
            ..Society::default()
        }
    }

    /// Society as `store` holds it for the game `content` is: every record of the kinds it keeps
    /// whole in memory, and how many messages each inbox holds. Refused when the store cannot be
    /// read, or holds a record this build does not read whole. It reads on the runtime it is
    /// called on.
    pub async fn read(
        store: &RecordStore,
        content: &Content,
        sources: Vec<SocialRequestSource>,
    ) -> Result<Society, RecordStoreProblem> {
        let scope = &ContentScope::of(content);
        let reads = match content.built::<GameSettings>().messages {
            Some(_) => InboxReads::from(
                store.clone(),
                tokio::runtime::Handle::current(),
                scope.clone(),
            ),
            None => InboxReads::default(),
        };
        let messages = all::<InboxMessage>(store, &scope.clone().setting_aside()).await?;
        Ok(Society {
            accounts: Accounts::of(all::<Account>(store, scope).await?),
            friends: Friendships::of(all::<Friendship>(store, scope).await?),
            blocks: BlockLists::of(all::<Block>(store, scope).await?),
            requests: SocialRequests::of(all::<SocialRequest>(store, scope).await?),
            inboxes: Inboxes::counted(messages, reads),
            parties: PlayerParties::of(all::<PlayerParty>(store, scope).await?),
            sources,
            ..Society::default()
        })
    }

    pub fn accounts(&self) -> &Accounts {
        &self.accounts
    }

    pub fn friends(&self) -> &Friendships {
        &self.friends
    }

    pub fn blocks(&self) -> &BlockLists {
        &self.blocks
    }

    pub fn requests(&self) -> &SocialRequests {
        &self.requests
    }

    pub fn inboxes(&self) -> &Inboxes {
        &self.inboxes
    }

    pub fn parties(&self) -> &PlayerParties {
        &self.parties
    }

    /// Who `account` takes requests from, in a game with these `settings`.
    pub fn accepts(&self, account: &AccountId, settings: &RequestSettings) -> AcceptsRequestsFrom {
        self.accounts
            .get(account)
            .and_then(|account| account.accepts_requests_from)
            .unwrap_or(settings.new_accounts_accept)
    }

    /// Whether `asked` takes requests from `asker`, by who they chose to take them from.
    pub fn admits(&self, asked: &AccountId, asker: &AccountId, settings: &RequestSettings) -> bool {
        match self.accepts(asked, settings) {
            AcceptsRequestsFrom::Everyone => true,
            AcceptsRequestsFrom::Friends => self.friends.are(asked, asker),
            AcceptsRequestsFrom::Party => self.parties.together(asked, asker),
            AcceptsRequestsFrom::Nobody => false,
        }
    }

    /// `account` as other players know it.
    pub fn member(&self, account: &AccountId) -> Option<SocietyMember> {
        Some(SocietyMember {
            account: self.accounts.number(account)?,
            name: self.accounts.get(account)?.name.clone(),
        })
    }

    /// The area the player of `account` plays in, while they are at their character.
    pub fn playing_in(&self, account: &AccountId) -> Option<AreaId> {
        seen_in(&self.playing, account)
    }

    pub(crate) fn source(&self, kind: SocialRequestKind) -> Option<SocialRequestSource> {
        self.sources
            .iter()
            .find(|source| source.kind == kind)
            .copied()
    }

    /// Notes that what the player of `account` sees of society changed.
    pub(crate) fn changed(&mut self, account: &AccountId) {
        self.changed.insert(account.clone());
    }
}

impl Society {
    /// Notes who plays where as `roster` lists them, so everyone who sees a player come, go or
    /// change areas is shown it, an account whose player left is saved as last seen `now`, and
    /// memory holds the inbox of every account that plays.
    fn note_playing(&mut self, roster: &ServerRoster, unwritten: &mut Unwritten, now: UnixMillis) {
        let playing: HashMap<AccountId, Playing> = roster
            .all()
            .into_iter()
            .filter_map(|enrolled| {
                let playing = Playing {
                    area: enrolled.place.area,
                    away: enrolled.away,
                };
                Some((enrolled.account?, playing))
            })
            .collect();
        if playing != self.playing {
            let before = std::mem::replace(&mut self.playing, playing);
            let moved: BTreeSet<&AccountId> = before
                .keys()
                .chain(self.playing.keys())
                .filter(|&account| seen_in(&before, account) != seen_in(&self.playing, account))
                .collect();
            for account in moved {
                if seen_in(&self.playing, account).is_none() {
                    let left = self
                        .accounts
                        .change(account, |account| account.last_seen = now);
                    if let Some(left) = left.cloned() {
                        unwritten.put(left);
                    }
                } else if seen_in(&before, account).is_none() {
                    self.inboxes.arrived(account);
                }
                let friends = self.friends.of_account(account);
                let party = self.parties.of_account(account);
                let watching: Vec<AccountId> = friends
                    .chain(party.into_iter().flat_map(PlayerParty::accounts))
                    .cloned()
                    .collect();
                self.changed.extend(watching);
            }
        }
        let playing = &self.playing;
        self.inboxes
            .keep(|account| playing.contains_key(account), unwritten);
    }
}

/// The area the player of `account` plays in, while they are at their character.
fn seen_in(playing: &HashMap<AccountId, Playing>, account: &AccountId) -> Option<AreaId> {
    playing
        .get(account)
        .filter(|playing| !playing.away)
        .map(|playing| playing.area)
}

/// The server's society, as every world of the server holds it. A world reads it as it stood
/// when the worlds last conferred, and never changes it: it asks, and society decides.
#[derive(Resource, Clone, Default)]
pub struct ServerSociety(Arc<RwLock<Society>>);

impl ServerSociety {
    pub fn of(society: Society) -> ServerSociety {
        ServerSociety(Arc::new(RwLock::new(society)))
    }

    /// Society as it stands. It only changes while the worlds confer, when no world reads it.
    pub fn read(&self) -> RwLockReadGuard<'_, Society> {
        self.0
            .try_read()
            .expect("society is read between two conferences, never during one")
    }

    /// Notes that the account of `identity` signed in at `now`.
    pub fn signed_in(
        &self,
        identity: &AccountIdentity,
        now: UnixMillis,
        unwritten: &mut Unwritten,
    ) {
        let mut society = self.change();
        let account = society
            .accounts
            .signed_in(&identity.id, &identity.name, now);
        unwritten.put(account.clone());
    }

    fn is(&self, other: &ServerSociety) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    fn change(&self) -> RwLockWriteGuard<'_, Society> {
        self.0.write().expect("society lock")
    }
}

/// What a world asks of society, on behalf of one of its players.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SocietyAsk {
    Request(SocialRequestAsk),
    Block(BlockAsk),
    Accepts(AcceptsRequestsFrom),
    Friend(FriendAsk),
    Inbox(InboxAsk),
    Party(PartyRequest),
}

/// What a world's players asked of society since the server's worlds last conferred.
#[derive(Resource, Default)]
pub struct SocietyAsks(Vec<(ClientId, SocietyAsk)>);

/// Asks society for `ask`, for the player `by`. It is settled when the worlds next confer.
pub fn ask(world: &mut World, by: ClientId, ask: SocietyAsk) {
    world.resource_mut::<SocietyAsks>().0.push((by, ask));
}

/// Society being settled, with every world of the server in hand: what a kind of request works
/// with as it is accepted. Whatever runs here has society in `society`, and must not read the
/// one a world holds.
pub struct Settling<'a, 'w> {
    pub society: &'a mut Society,
    pub views: &'a mut [Option<&'w mut World>],
    pub roster: &'a ServerRoster,
    pub unwritten: &'a mut Unwritten,
    pub now: UnixMillis,
}

impl Settling<'_, '_> {
    /// The world the character of `account` stands in, and the character.
    pub fn reach(&mut self, account: &AccountId) -> Option<(&mut World, Entity)> {
        reach(self.views, self.roster, account)
    }

    /// What `account` is called.
    pub fn name(&self, account: &AccountId) -> String {
        self.society
            .accounts
            .get(account)
            .map(|account| account.name.clone())
            .unwrap_or_default()
    }

    /// Tells the player of `account` `notification`, when they are playing.
    pub fn tell(&mut self, account: &AccountId, notification: impl Into<Notification>) {
        if let Some((world, character)) = self.reach(account) {
            notification::notify(world, character, None, notification);
        }
    }

    /// Sends the client of `account` `message`, when they are playing.
    pub fn send<M: Message>(&mut self, account: &AccountId, message: M) {
        let Some((world, character)) = self.reach(account) else {
            return;
        };
        let conn = world
            .get::<PlayedBy>(character)
            .and_then(|by| world.resource::<Connected>().connection(by.client));
        if let Some(conn) = conn {
            visibility::send_to(world, conn, message);
        }
    }

    /// Tells the player of `account` that what they asked for was refused, and why.
    pub fn refuse(&mut self, account: &AccountId, why: String) {
        if let Some((world, character)) = self.reach(account) {
            notification::error(world, character, why);
        }
    }
}

/// In `worlds::confer`, once the roster lists who plays: takes what every world's players asked
/// of society since, world by world and each world's in the order they were asked, changes
/// society, has the next checkpoint write each record it changed, and shows every player
/// playing what they now see of it.
pub fn settle(views: &mut [Option<&mut World>], above: &mut AboveWorlds, now: UnixMillis) {
    let mut asked: Vec<(Content, ClientId, SocietyAsk)> = Vec::new();
    let mut game: Option<Content> = None;
    for world in views.iter_mut().flatten() {
        if !world.contains_resource::<SocietyAsks>() {
            continue;
        }
        if game.is_none() {
            game = world.get_resource::<Content>().cloned();
        }
        if !world.resource::<ServerSociety>().is(&above.society) {
            world.insert_resource(above.society.clone());
        }
        let asks = std::mem::take(&mut world.resource_mut::<SocietyAsks>().0);
        if asks.is_empty() {
            continue;
        }
        let content = world.resource::<Content>().clone();
        asked.extend(asks.into_iter().map(|(by, ask)| (content.clone(), by, ask)));
    }
    let mut society = above.society.change();
    society.note_playing(&above.roster, &mut above.checkpoints.unwritten, now);
    let mut settling = Settling {
        society: &mut society,
        views,
        roster: &above.roster,
        unwritten: &mut above.checkpoints.unwritten,
        now,
    };
    for (content, by, ask) in asked {
        let Some(by) = settling.roster.player(by).and_then(|player| player.account) else {
            continue;
        };
        let settings = content.built::<GameSettings>();
        match ask {
            SocietyAsk::Request(ask) => social_request::settle(&mut settling, settings, &by, ask),
            SocietyAsk::Block(ask) => {
                if let Some(blocked) = social_request::block::settle(&mut settling, &by, ask) {
                    friend::part(&mut settling, &by, &blocked);
                }
            }
            SocietyAsk::Friend(ask) => friend::settle(&mut settling, settings, &by, ask),
            SocietyAsk::Inbox(ask) => inbox::settle(&mut settling, settings, &by, ask),
            SocietyAsk::Party(ask) => party::settle(&mut settling, settings, &by, ask),
            SocietyAsk::Accepts(from) => {
                let chosen = settling.society.accounts.change(&by, |account| {
                    account.accepts_requests_from = Some(from);
                });
                if let Some(account) = chosen.cloned() {
                    settling.unwritten.put(account);
                    settling.society.changed(&by);
                }
            }
        }
    }
    if let Some(content) = game {
        let settings = content.built::<GameSettings>();
        social_request::lapse(&mut settling, settings);
        party::keep(&mut settling, settings);
    }
    for account in std::mem::take(&mut settling.society.changed) {
        if let Some((world, character)) = reach(settling.views, settling.roster, &account) {
            view::show(world, character, settling.society);
        }
    }
    for account in settling.society.inboxes.changed() {
        if let Some((world, character)) = reach(settling.views, settling.roster, &account) {
            inbox::show(world, character, &account, settling.society);
        }
    }
}

fn reach<'v>(
    views: &'v mut [Option<&mut World>],
    roster: &ServerRoster,
    account: &AccountId,
) -> Option<(&'v mut World, Entity)> {
    let client = roster.account(account)?.client;
    views.iter_mut().flatten().find_map(|world| {
        let character = *world.get_resource::<Players>()?.0.get(&client)?;
        Some((&mut **world, character))
    })
}

async fn all<R: Record>(
    store: &RecordStore,
    scope: &ContentScope,
) -> Result<Vec<R>, RecordStoreProblem> {
    store
        .all(R::KIND)
        .await?
        .iter()
        .map(|body| store::read(body, scope))
        .collect()
}
