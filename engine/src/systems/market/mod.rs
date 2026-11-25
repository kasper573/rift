//! The market: one for the whole server. A seller lists goods for a price in items,
//! each with how many, for one of the game's lengths, and the first player to pay the price
//! gets the goods. The market holds a listing's goods from the moment it is listed, so
//! every way a listing ends pays out what it holds, in the same settle that ends it.

pub mod effect;
pub mod window;

use crate::core::time;
use crate::systems::player::session;
use crate::systems::society::SocietyView;
use std::collections::{BTreeMap, BTreeSet};

use bevy_app::App;
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::{Content, ContentProblem, Loading, SETTINGS_FILE, SavedList};
use crate::core::script::{self, OwnerRow, Scripts};
use crate::core::store::{Record, RecordId};
use crate::core::text::LineText;
use crate::core::time::UnixMillis;
use crate::core::units::Seconds;
use crate::systems::GameSettings;
use crate::systems::account::AccountId;
use crate::systems::history::HistoryTopic;
use crate::systems::item::deliver::{self, DeliveryReason};
use crate::systems::item::{ExchangeRefusal, Inventory, ItemDef, ItemId, ItemStack};
use crate::systems::mention::Mention;
use crate::systems::messages::{self, messages};
use crate::systems::notification::Notification;
use crate::systems::player::{self, PlayedBy, RequestNeeds};
use crate::systems::reach::{self, Tether};
use crate::systems::society::{self, Settling, Society, SocietyAsk};
use crate::systems::window::{AnswerCount, OpenWindow};

messages! {
    /// What the game says of the market.
    pub struct MarketMessages {
        /// What the player is told once their listing of `{goods}` is up on the market.
        listed[goods: Line],
        /// What a seller is told as their listing of `{goods}` is bought and they get `{price}`.
        sold[goods: Line, price: Line],
        /// What a seller is told as their listing of `{goods}` ends unbought.
        expired[goods: Line],
        /// What a seller is told as `{goods}` comes back from a listing they cancelled.
        cancelled[goods: Line],
        /// What a player is told as someone else bought the listing of `{goods}` before their
        /// purchase settled.
        gone[goods: Line],
        /// What a player is told listing more than the `{listings}` listings they may have up.
        too_many(listings: Number),
        /// The text of every message the market sends.
        message,
        /// The title of the message of what a purchase on the market brought that had no room.
        bought_title,
        /// The title of the message paying a seller for their listing.
        sold_title,
        /// The title of the message bringing back the goods of a listing.
        returned_title,
        /// What a player is told listing no goods.
        no_goods,
        /// What a player is told listing without setting a price.
        no_price,
        /// What a player is told listing more than the `{items}` items a listing's goods may hold.
        goods_too_many(items: Number),
        /// What a player is told pricing in more than the `{items}` items a listing's price may hold.
        price_too_many(items: Number),
        /// What a player is told naming the same item twice on one side of a listing.
        named_twice,
        /// What a player is told naming `{item}` without how many.
        needs_amount[item: Line],
        /// The market window's title.
        title,
        /// The tab to search and buy on.
        browse,
        /// The tab to put items up on.
        new_listing,
        /// The tab of the player's own listings, `{count}` of them up.
        my_listings(count: Number),
        /// The label over the field naming the item to look for or to price in.
        item,
        /// What the item field invites before an item is chosen, and as it is typed in.
        item_hint,
        /// What the item field's list says when no item's name holds what was typed.
        no_such_item,
        /// What Browse says before an item is chosen.
        choose_item,
        /// What Browse says when no listing has `{item}` among its goods.
        no_listings_of[item: Line],
        /// The label over how the listings are sorted.
        sort_by,
        /// The label over which way the listings are sorted.
        order,
        /// The listings sorted from the highest to the lowest.
        high_to_low,
        /// The listings sorted from the lowest to the highest.
        low_to_high,
        /// Where Browse is among its pages: page `{page}` of `{pages}`, `{listings}` listings in all.
        page(listings: Number, page: Number, pages: Number),
        /// The heading over the goods of listings.
        goods,
        /// The heading over what listings cost.
        price,
        /// A listing's price for one of the item sorted by, `{price}`.
        each[price: Line],
        /// The button that buys a listing.
        buy,
        /// What a listing someone else bought first says of itself.
        taken,
        /// What the player's own listing says of itself, `{left}` its time left.
        yours(left: Text),
        /// Why Buy is shut on a listing someone else bought first.
        bought_first,
        /// Why Buy is shut on the player's own listing.
        own_listing,
        /// Why Buy is shut on a listing whose price the player can't pay.
        cannot_pay,
        /// What the player is asked before buying `{goods}` for `{price}`.
        confirm_buy[goods: Line, price: Line],
        /// What the player is reminded of before buying.
        cannot_undo,
        /// The heading over the goods of the new listing.
        your_goods,
        /// The heading over the new listing's price.
        your_price,
        /// The label over how many of an item a price asks for.
        amount,
        /// The button adding another item to the new listing's price.
        add_item,
        /// The label before how long the new listing stays up.
        lasts,
        /// What the new listing costs to put up: `{deposit}`, kept unless it is bought.
        deposit[deposit: Line],
        /// The button that puts the new listing up.
        create,
        /// What My listings says while the player has none up.
        none_up,
        /// What My listings says of the player's `{count}` listings, of the `{most}` they may have.
        up_of(count: Number, most: Number),
        /// What a listing of the player's holds, `{goods}`.
        holds[goods: Line],
        /// What a listing of the player's costs, `{price}`.
        costs[price: Line],
    }
}

pub fn register(app: &mut App) {
    use bevy_replication::{AppMessageExt, AppReplicationExt};
    app.replicate_private::<MarketView>()
        .add_client_message::<MarketRequest>();
}

/// How a game's market works. A game without it has none.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarketSettings {
    /// The item a new listing's first price row starts on. A listing's goods and price may hold any
    /// items that can change hands.
    pub default_currency: ItemId,
    /// How many different items a listing's goods may hold.
    pub max_goods_items: u32,
    /// How many different items a listing's price may hold.
    pub max_price_items: u32,
    /// The lengths a seller may choose from.
    pub durations: Vec<Seconds>,
    /// The item the deposit is paid in.
    pub deposit_currency: ItemId,
    /// How many of the deposit currency a listing costs, kept by the market unless the listing
    /// is taken.
    pub deposit: u32,
    /// How many listings an account may have up at once.
    pub max_listings: u32,
    /// The most listings a page of results shows, however many a player's window has room for.
    pub page_rows: u32,
}

impl MarketSettings {
    /// What listing costs.
    pub fn deposit(&self) -> ItemStack {
        ItemStack::new(self.deposit_currency, self.deposit)
    }

    /// What a listing of `goods` costs its seller: the goods, and the deposit unless it is none.
    pub fn costs(&self, goods: &[ItemStack]) -> Vec<ItemStack> {
        goods.iter().chain(&self.deposit_paid()).copied().collect()
    }

    /// The deposit a listing is put up with, none when it costs none.
    pub fn deposit_paid(&self) -> Vec<ItemStack> {
        Some(self.deposit())
            .filter(|deposit| deposit.count > 0)
            .into_iter()
            .collect()
    }

    /// Why the market refuses a listing of `goods` for `price` from a seller with `bag` who has
    /// `listed` listings up, if it does: a side with none or too many items, an item named twice
    /// or none of, or one that can't change hands; too many listings up; or a bag that doesn't
    /// hold what the listing costs.
    pub fn refusal(
        &self,
        content: &Content,
        bag: &Inventory,
        listed: u32,
        goods: &[ItemStack],
        price: &[ItemStack],
    ) -> Option<ListingRefusal> {
        if let Some(refusal) = self
            .side_refusal(content, ListingSide::Goods, goods)
            .or_else(|| self.side_refusal(content, ListingSide::Price, price))
        {
            return Some(refusal);
        }
        if listed >= self.max_listings {
            return Some(ListingRefusal::TooManyListings(self.max_listings));
        }
        bag.clone()
            .exchange(content, &self.costs(goods), &[])
            .err()
            .map(ListingRefusal::Costs)
    }

    fn side_refusal(
        &self,
        content: &Content,
        side: ListingSide,
        stacks: &[ItemStack],
    ) -> Option<ListingRefusal> {
        let most = match side {
            ListingSide::Goods => self.max_goods_items,
            ListingSide::Price => self.max_price_items,
        };
        if stacks.is_empty() {
            return Some(ListingRefusal::Empty(side));
        }
        if stacks.len() as u32 > most {
            return Some(ListingRefusal::TooMany(side, most));
        }
        if let Some(stack) = stacks.iter().find(|stack| stack.count == 0) {
            return Some(ListingRefusal::NoneOf(stack.item));
        }
        let mut items: Vec<ItemId> = stacks.iter().map(|stack| stack.item).collect();
        items.sort();
        if items.windows(2).any(|pair| pair[0] == pair[1]) {
            return Some(ListingRefusal::Twice);
        }
        stacks
            .iter()
            .find(|stack| stack.bound(content))
            .map(|stack| ListingRefusal::Bound(stack.item))
    }
}

/// One of the two sets of items a listing holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListingSide {
    Goods,
    Price,
}

/// Why one side of a listing is not one the market takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListingRefusal {
    Empty(ListingSide),
    TooMany(ListingSide, u32),
    NoneOf(ItemId),
    Twice,
    Bound(ItemId),
    TooManyListings(u32),
    Costs(ExchangeRefusal),
}

impl ListingRefusal {
    pub fn describe(self, content: &Content) -> LineText {
        let words = &messages::of(content).market;
        match self {
            ListingRefusal::Empty(ListingSide::Goods) => LineText::plain(words.no_goods()),
            ListingRefusal::Empty(ListingSide::Price) => LineText::plain(words.no_price()),
            ListingRefusal::TooMany(ListingSide::Goods, items) => {
                LineText::plain(words.goods_too_many(items))
            }
            ListingRefusal::TooMany(ListingSide::Price, items) => {
                LineText::plain(words.price_too_many(items))
            }
            ListingRefusal::NoneOf(item) => words.needs_amount(Mention::Item(item).span(content)),
            ListingRefusal::Twice => LineText::plain(words.named_twice()),
            ListingRefusal::Bound(item) => ItemStack::bound_line(item, content),
            ListingRefusal::TooManyListings(most) => LineText::plain(words.too_many(most)),
            ListingRefusal::Costs(refusal) => refusal.line(content),
        }
    }
}

#[derive(
    JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
pub struct ListingId(pub u64);

/// Items up on the market for other items.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Listing {
    pub id: ListingId,
    pub seller: AccountId,
    /// The goods of the listing, held by the market. An item the game no longer has waits set
    /// aside, and the listing can't be taken until it is back.
    pub goods: SavedList<ItemStack>,
    /// The seller's price for all of it. An item the game no longer has waits set aside, and
    /// the listing can't be taken until it is back.
    pub price: SavedList<ItemStack>,
    /// What listing it cost, given back if it is taken.
    pub deposit: SavedList<ItemStack>,
    pub ends: UnixMillis,
}

impl Record for Listing {
    const KIND: &'static str = "listing";

    fn id(&self) -> RecordId {
        RecordId(self.id.0.to_string())
    }
}

impl Listing {
    /// Its goods, while the game has every item of them.
    pub fn known_goods(&self) -> Option<&[ItemStack]> {
        self.goods.set_aside().is_empty().then_some(&self.goods[..])
    }

    /// Its price, while the game has every item of it.
    pub fn known_price(&self) -> Option<&[ItemStack]> {
        self.price.set_aside().is_empty().then_some(&self.price[..])
    }
}

/// Every listing up on the server, indexed so what a player asks of the market costs as much as
/// what they are shown: by when each ends, by seller, by every item among its goods, and by every way
/// the listings with an item among their goods can be sorted.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Market {
    listings: BTreeMap<ListingId, Listing>,
    ends: BTreeSet<(UnixMillis, ListingId)>,
    by_seller: BTreeMap<AccountId, BTreeSet<(UnixMillis, ListingId)>>,
    /// The listings that can be bought with each item among their goods, those ending first first.
    with_goods: BTreeMap<ItemId, BTreeSet<(UnixMillis, ListingId)>>,
    /// The same listings by each way they can be sorted, the least first.
    ranked: BTreeMap<ItemId, BTreeMap<SortBy, BTreeSet<Ranked>>>,
    dealt: u64,
}

impl Market {
    /// The market holding `saved`.
    pub fn of(saved: impl IntoIterator<Item = Listing>) -> Market {
        let mut market = Market::default();
        for listing in saved {
            market.dealt = market.dealt.max(listing.id.0);
            market.hold(listing);
        }
        market
    }

    pub fn get(&self, id: ListingId) -> Option<&Listing> {
        self.listings.get(&id)
    }

    /// How many listings are up.
    pub fn len(&self) -> usize {
        self.listings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.listings.is_empty()
    }

    /// The listings `seller` has up, those ending first first.
    pub fn of_seller(&self, seller: &AccountId) -> impl Iterator<Item = &Listing> {
        self.by_seller
            .get(seller)
            .into_iter()
            .flatten()
            .filter_map(|(_, id)| self.listings.get(id))
    }

    /// Puts `listing` up under the next id.
    fn add(&mut self, listing: Listing) -> Listing {
        self.dealt += 1;
        let listing = Listing {
            id: ListingId(self.dealt),
            ..listing
        };
        self.hold(listing.clone());
        listing
    }

    fn hold(&mut self, listing: Listing) {
        let ending = (listing.ends, listing.id);
        self.ends.insert(ending);
        self.by_seller
            .entry(listing.seller.clone())
            .or_default()
            .insert(ending);
        for (goods, ranks) in Self::ranks(&listing) {
            self.with_goods.entry(goods).or_default().insert(ending);
            for (by, ranked) in ranks {
                self.ranked
                    .entry(goods)
                    .or_default()
                    .entry(by)
                    .or_default()
                    .insert(ranked);
            }
        }
        self.listings.insert(listing.id, listing);
    }

    fn remove(&mut self, id: ListingId) -> Option<Listing> {
        let listing = self.listings.remove(&id)?;
        let ending = (listing.ends, id);
        self.ends.remove(&ending);
        if let Some(selling) = self.by_seller.get_mut(&listing.seller) {
            selling.remove(&ending);
            if selling.is_empty() {
                self.by_seller.remove(&listing.seller);
            }
        }
        for (goods, ranks) in Self::ranks(&listing) {
            if let Some(with_goods) = self.with_goods.get_mut(&goods) {
                with_goods.remove(&ending);
                if with_goods.is_empty() {
                    self.with_goods.remove(&goods);
                }
            }
            let Some(sorts) = self.ranked.get_mut(&goods) else {
                continue;
            };
            for (by, ranked) in ranks {
                if let Some(sorted) = sorts.get_mut(&by) {
                    sorted.remove(&ranked);
                    if sorted.is_empty() {
                        sorts.remove(&by);
                    }
                }
            }
            if sorts.is_empty() {
                self.ranked.remove(&goods);
            }
        }
        Some(listing)
    }

    /// The first listing whose time is up at `now`.
    fn ended(&self, now: UnixMillis) -> Option<ListingId> {
        self.ends
            .first()
            .filter(|(ends, _)| *ends <= now)
            .map(|&(_, id)| id)
    }

    /// Where `listing` stands in the market while it can be bought, while the game has every
    /// item of its goods and price: under each item of its goods, by every way it can be sorted
    /// there.
    fn ranks(listing: &Listing) -> Vec<(ItemId, Vec<(SortBy, Ranked)>)> {
        let (Some(goods), Some(price)) = (listing.known_goods(), listing.known_price()) else {
            return Vec::new();
        };
        let ranked = |amount: u32, per: u32| Ranked {
            amount,
            per,
            ends: listing.ends,
            id: listing.id,
        };
        goods
            .iter()
            .map(|found| {
                let by_goods = goods
                    .iter()
                    .map(|stack| (SortBy::Goods(stack.item), ranked(stack.count, 1)));
                let by_price = price.iter().map(|stack| {
                    let per = found.count.max(1);
                    (SortBy::Price(stack.item), ranked(stack.count, per))
                });
                (found.item, by_goods.chain(by_price).collect())
            })
            .collect()
    }

    /// The item in the prices of the most listings with `item` among their goods, the first of those that tie.
    fn most_common_price(&self, item: ItemId) -> Option<ItemId> {
        self.ranked
            .get(&item)?
            .iter()
            .rev()
            .filter_map(|(by, ranked)| match by {
                SortBy::Price(price) => Some((*price, ranked.len())),
                SortBy::Goods(_) => None,
            })
            .max_by_key(|&(_, listings)| listings)
            .map(|(price, _)| price)
    }

    /// The listings with `item` among their goods, in the order `sort` gives: those it can't rank after,
    /// ending first first.
    fn found(&self, item: ItemId, sort: Option<ListingSort>) -> impl Iterator<Item = ListingId> {
        let sorted = sort.and_then(|sort| Some((self.ranked.get(&item)?.get(&sort.by)?, sort)));
        let ranked: Box<dyn Iterator<Item = ListingId>> = match sorted {
            Some((sorted, sort)) if sort.descending => {
                Box::new(sorted.iter().rev().map(|ranked| ranked.id))
            }
            Some((sorted, _)) => Box::new(sorted.iter().map(|ranked| ranked.id)),
            None => Box::new(std::iter::empty()),
        };
        let unranked = move |id: &ListingId| {
            sorted.is_none_or(|(_, sort)| {
                self.listings
                    .get(id)
                    .is_none_or(|listing| !sort.by.ranks(listing))
            })
        };
        let rest = self
            .with_goods
            .get(&item)
            .into_iter()
            .flatten()
            .map(|&(_, id)| id)
            .filter(move |id| unranked(id));
        ranked.chain(rest)
    }
}

/// Where a listing stands among those sorted one way: by the `amount` it holds, as goods or in its price,
/// of the item sorted by, for `per` of the item it is found under, the least first, then those ending
/// first.
#[derive(Clone, Copy, Debug)]
struct Ranked {
    amount: u32,
    per: u32,
    ends: UnixMillis,
    id: ListingId,
}

impl Ord for Ranked {
    fn cmp(&self, other: &Ranked) -> std::cmp::Ordering {
        let mine = u64::from(self.amount) * u64::from(other.per);
        let theirs = u64::from(other.amount) * u64::from(self.per);
        mine.cmp(&theirs)
            .then(self.ends.cmp(&other.ends))
            .then(self.id.cmp(&other.id))
    }
}

impl PartialOrd for Ranked {
    fn partial_cmp(&self, other: &Ranked) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Ranked {
    fn eq(&self, other: &Ranked) -> bool {
        self.cmp(other).is_eq()
    }
}

impl Eq for Ranked {}

/// What a player asks of the market.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum MarketAsk {
    Search(MarketQuery),
    List {
        goods: Vec<ItemStack>,
        price: Vec<ItemStack>,
        duration: Seconds,
    },
    /// Buys the listing, as the player saw it: one whose seller changed it since is not bought.
    Buy {
        listing: ListingId,
        goods: Vec<ItemStack>,
        price: Vec<ItemStack>,
    },
    Cancel(ListingId),
}

/// What a player asks of the market window.
#[derive(JsonSchema, Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum MarketRequest {
    Ask(MarketAsk),
    Close,
}

/// A search of the market: the listings with `item` among their goods, in the order `sort` gives, `rows` to a
/// page.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct MarketQuery {
    pub item: ItemId,
    /// When none is asked for, sorts by the item in the most of the listings' prices, the lowest
    /// first.
    pub sort: Option<ListingSort>,
    pub page: u32,
    pub rows: u32,
}

/// A sort of listings by what `by` names: the least first, or the most when `descending`.
/// Listings it can't rank come after.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListingSort {
    pub by: SortBy,
    pub descending: bool,
}

/// What listings are sorted by: how many of an item their goods hold, or their price in an item for
/// one of the item searched for.
#[derive(
    JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord,
)]
#[serde(rename_all = "snake_case")]
pub enum SortBy {
    Goods(ItemId),
    Price(ItemId),
}

impl SortBy {
    /// Whether `listing` has the item this sorts by, where it sorts by it.
    fn ranks(self, listing: &Listing) -> bool {
        let (stacks, item) = match self {
            SortBy::Goods(item) => (&listing.goods, item),
            SortBy::Price(item) => (&listing.price, item),
        };
        stacks.iter().any(|stack| stack.item == item)
    }
}

/// A listing as a player sees it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ListingRow {
    pub id: ListingId,
    pub goods: Vec<ItemStack>,
    pub price: Vec<ItemStack>,
    pub ends: UnixMillis,
    /// Whether the player it is shown to listed it.
    pub mine: bool,
}

/// What a search found: a page of the listings that match, with the search as it was answered:
/// its sort, and its page within those there are.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MarketResults {
    pub query: MarketQuery,
    pub rows: Vec<ListingRow>,
    /// How many listings match.
    pub found: u32,
    pub pages: u32,
    /// Every way the listings that match can be sorted: by each item among their goods, then by each
    /// item of their prices.
    pub sorts: Vec<SortBy>,
}

/// The market as the player who has it open sees it.
#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct MarketView {
    pub answered: AnswerCount,
    /// What the latest search found: none before the first.
    pub results: Option<MarketResults>,
    /// The listings of those results someone else bought before this player's purchase settled.
    pub gone: Vec<ListingId>,
}

/// Opens the market for `player`, from `opener` where a thing in the world opened it: then it
/// closes as they walk out of its reach.
pub fn open(world: &mut World, player: Entity, opener: Option<Entity>) {
    if world.resource::<GameSettings>().market.is_none() {
        return;
    }
    let tether = Tether::opened_by(world, player, opener);
    world
        .entity_mut(player)
        .insert((MarketVisit { tether }, MarketView::default()));
}

/// Hands what players of this world asked of the market to society.
pub fn requests(world: &mut World) {
    player::player_requests::<MarketRequest>(
        world,
        RequestNeeds::Nothing,
        |world, player, request| {
            match request {
                MarketRequest::Close => close(world, player),
                MarketRequest::Ask(ask) => {
                    let client = world.get::<PlayedBy>(player).map(|by| by.client);
                    if let (Some(client), true) =
                        (client, world.get::<MarketVisit>(player).is_some())
                    {
                        society::ask(world, client, SocietyAsk::Market(ask));
                    }
                }
            }
            Ok(())
        },
    );
}

/// Does what `by` asked of the market.
pub(crate) fn settle(settling: &mut Settling, content: &Content, by: &AccountId, ask: MarketAsk) {
    let Some(settings) = content.built::<GameSettings>().market.clone() else {
        return;
    };
    match ask {
        MarketAsk::Search(query) => search(settling, settings.page_rows, by, query),
        MarketAsk::List {
            goods,
            price,
            duration,
        } => list(settling, content, &settings, by, goods, price, duration),
        MarketAsk::Buy {
            listing,
            goods,
            price,
        } => buy(settling, content, by, listing, goods, price),
        MarketAsk::Cancel(listing) => cancel(settling, content, by, listing),
    }
    show(settling, by, |view| view.answered.raise());
}

/// Ends every listing whose time is up, its goods going back to its seller.
pub(crate) fn keep(settling: &mut Settling, content: &Content) {
    while let Some(id) = settling.society.market.ended(settling.now) {
        let Some(listing) = settling.society.market.remove(id) else {
            continue;
        };
        settling.unwritten.remove::<Listing>(listing.id());
        give_back(settling, content, listing, Returned::Expired);
    }
}

/// The listings `account` has up, as they see them.
pub(crate) fn mine(society: &Society, account: &AccountId) -> Vec<ListingRow> {
    society
        .market
        .of_seller(account)
        .filter_map(|listing| row(listing, account))
        .collect()
}

/// A game whose content opens the market has one, and the items its settings name can change
/// hands.
pub(crate) fn check(loading: &mut Loading, content: &Content, settings: &GameSettings) {
    if let Some(market) = &settings.market {
        let named = [
            ("/market/default_currency", market.default_currency),
            ("/market/deposit_currency", market.deposit_currency),
        ];
        for (pointer, item) in named {
            if content
                .table::<ItemDef>()
                .get(item)
                .is_some_and(ItemDef::bound)
            {
                let problem = ContentProblem::new(
                    "bound currency",
                    "a bound item",
                    "an item that can change hands",
                );
                loading.json_error(SETTINGS_FILE, pointer, problem);
            }
        }
    }
    if settings.market.is_some() {
        return;
    }
    let scripts = content.built::<Scripts>();
    let opening: Vec<OwnerRow> = scripts
        .owners()
        .map(|(row, _)| *row)
        .filter(|row| {
            scripts
                .may_do(*row)
                .reaching::<effect::OpensMarket>()
                .next()
                .is_some()
        })
        .collect();
    for row in opening {
        let site = script::owner_site(content, row);
        let problem = ContentProblem::new(
            "no market",
            "a script that opens the market",
            "market in the game's settings",
        );
        loading.json_error(&site.file, &site.pointer, problem);
    }
}

/// Where a player has the market open from.
#[derive(Component)]
pub struct MarketVisit {
    /// The thing that opened it, and how far from it the player may stand.
    tether: Option<Tether>,
}

/// How a listing ended, its goods going back to its seller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Returned {
    Expired,
    Cancelled,
}

impl OpenWindow for MarketVisit {
    fn kept(&self, world: &World, player: Entity) -> bool {
        reach::lapse(world, player, self.tether.as_ref()).is_none()
    }

    fn close(world: &mut World, player: Entity) {
        close(world, player);
    }
}

fn close(world: &mut World, player: Entity) {
    if let Ok(mut player) = world.get_entity_mut(player) {
        player.remove::<(MarketVisit, MarketView)>();
    }
}

fn search(settling: &mut Settling, page_rows: u32, by: &AccountId, query: MarketQuery) {
    let market = &settling.society.market;
    let sorts: Vec<SortBy> = market
        .ranked
        .get(&query.item)
        .map(|sorts| sorts.keys().copied().collect())
        .unwrap_or_default();
    let sort = query
        .sort
        .filter(|sort| sorts.contains(&sort.by))
        .or_else(|| {
            let by = SortBy::Price(market.most_common_price(query.item)?);
            Some(ListingSort {
                by,
                descending: false,
            })
        });
    let found = market.with_goods.get(&query.item).map_or(0, BTreeSet::len) as u32;
    let rows = query.rows.clamp(1, page_rows);
    let pages = found.div_ceil(rows).max(1);
    let page = query.page.min(pages - 1);
    let shown = market
        .found(query.item, sort)
        .skip((page * rows) as usize)
        .take(rows as usize)
        .filter_map(|id| row(market.get(id)?, by))
        .collect();
    let results = MarketResults {
        query: MarketQuery {
            item: query.item,
            sort,
            page,
            rows,
        },
        rows: shown,
        found,
        pages,
        sorts,
    };
    show(settling, by, |view| {
        view.results = Some(results);
        view.gone.clear();
    });
}

fn list(
    settling: &mut Settling,
    content: &Content,
    settings: &MarketSettings,
    by: &AccountId,
    goods: Vec<ItemStack>,
    price: Vec<ItemStack>,
    duration: Seconds,
) {
    if !settings.durations.contains(&duration) {
        return;
    }
    let listed = settling.society.market.of_seller(by).count() as u32;
    let Some((world, character)) = settling.reach(by) else {
        return;
    };
    let refusal = settings.refusal(
        content,
        &Inventory::of(world, character),
        listed,
        &goods,
        &price,
    );
    if let Some(refusal) = refusal {
        return settling.refuse(by, refusal.describe(content));
    }
    let Some(mut bag) = world.get_mut::<Inventory>(character) else {
        return;
    };
    if bag.exchange(content, &settings.costs(&goods), &[]).is_err() {
        return;
    }
    let listing = settling.society.market.add(Listing {
        id: ListingId(0),
        seller: by.clone(),
        goods: goods.into(),
        price: price.into(),
        deposit: settings.deposit_paid().into(),
        ends: settling.now.after(duration),
    });
    settling.unwritten.put(listing);
    settling.society.changed(by);
}

fn buy(
    settling: &mut Settling,
    content: &Content,
    by: &AccountId,
    id: ListingId,
    goods: Vec<ItemStack>,
    price: Vec<ItemStack>,
) {
    let messages = messages::of(content);
    let unchanged = settling.society.market.get(id).is_some_and(|listing| {
        listing.known_goods() == Some(goods.as_slice())
            && listing.known_price() == Some(price.as_slice())
            && listing.seller != *by
    });
    if !unchanged {
        if settling.society.market.get(id).is_none() {
            let said = messages
                .market
                .gone(ItemStack::listed(&messages.interface, &goods));
            settling.refuse(by, said);
            show(settling, by, |view| view.gone.push(id));
        }
        return;
    }
    let Some((world, character)) = settling.reach(by) else {
        return;
    };
    let given = world
        .get_mut::<Inventory>(character)
        .map(|mut bag| bag.exchange(content, &price, &[]));
    match given {
        Some(Ok(())) => {}
        Some(Err(refusal)) => return settling.refuse(by, refusal.line(content)),
        None => return,
    }
    let Some(listing) = settling.society.market.remove(id) else {
        return;
    };
    settling.unwritten.remove::<Listing>(listing.id());
    deliver::to_account(settling, content, by, &goods, DeliveryReason::MarketBought);
    let paid: Vec<ItemStack> = price
        .iter()
        .chain(listing.deposit.iter())
        .copied()
        .collect();
    deliver::to_account(
        settling,
        content,
        &listing.seller,
        &paid,
        DeliveryReason::MarketSold,
    );
    let said = messages.market.sold(
        ItemStack::listed(&messages.interface, &goods),
        ItemStack::listed(&messages.interface, &price),
    );
    settling.tell(
        &listing.seller,
        Notification::feed(HistoryTopic::Item, said),
    );
    settling.society.changed(by);
    settling.society.changed(&listing.seller);
}

fn cancel(settling: &mut Settling, content: &Content, by: &AccountId, id: ListingId) {
    let own = settling
        .society
        .market
        .get(id)
        .is_some_and(|listing| listing.seller == *by);
    if !own {
        return;
    }
    let Some(listing) = settling.society.market.remove(id) else {
        return;
    };
    settling.unwritten.remove::<Listing>(listing.id());
    give_back(settling, content, listing, Returned::Cancelled);
}

/// Pays the goods of a listing that was not taken back to its seller, wherever they are: one with
/// an item set aside goes by message, as it was saved.
fn give_back(settling: &mut Settling, content: &Content, listing: Listing, why: Returned) {
    let messages = messages::of(content);
    let seller = listing.seller.clone();
    settling.society.changed(&seller);
    let Some(goods) = listing.known_goods() else {
        let reason = DeliveryReason::MarketReturned;
        deliver::by_mail(settling, content, &seller, listing.goods, reason);
        return;
    };
    deliver::to_account(
        settling,
        content,
        &seller,
        goods,
        DeliveryReason::MarketReturned,
    );
    let kept = ItemStack::listed(&messages.interface, goods);
    let said = match why {
        Returned::Expired => messages.market.expired(kept),
        Returned::Cancelled => messages.market.cancelled(kept),
    };
    settling.tell(&seller, Notification::feed(HistoryTopic::Item, said));
}

/// Changes `by`'s window as `edit` says, while they have the market open.
fn show(settling: &mut Settling, by: &AccountId, edit: impl FnOnce(&mut MarketView)) {
    let Some((world, character)) = settling.reach(by) else {
        return;
    };
    if let Some(mut view) = world.get_mut::<MarketView>(character) {
        edit(&mut view);
    }
}

fn row(listing: &Listing, viewer: &AccountId) -> Option<ListingRow> {
    Some(ListingRow {
        id: listing.id,
        goods: listing.known_goods()?.to_vec(),
        price: listing.known_price()?.to_vec(),
        ends: listing.ends,
        mine: listing.seller == *viewer,
    })
}

/// The market as it is open to the player: what their searches found, and their own listings.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ShownMarket {
    /// The page of listings the latest search found, none before the first.
    pub results: Option<Vec<ShownListing>>,
    pub sort: Option<ListingSort>,
    /// Every way the listings that match can be sorted.
    pub sorts: Vec<SortBy>,
    pub page: u32,
    pub pages: u32,
    /// How many listings match.
    pub found: u32,
    /// The listings the player has up.
    pub mine: Vec<ShownListing>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ShownListing {
    pub id: ListingId,
    pub goods: Vec<ShownMarketStack>,
    pub price: Vec<ShownMarketStack>,
    pub left: Seconds,
    pub mine: bool,
    /// Whether someone else bought it before the player's purchase settled.
    pub gone: bool,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq)]
pub struct ShownMarketStack {
    pub item: ItemId,
    pub count: u32,
}

pub fn shown(world: &World) -> Option<ShownMarket> {
    let view = view(world)?;
    let now = time::server_now(world)?;
    Some(ShownMarket {
        results: found(world),
        page: view
            .results
            .as_ref()
            .map_or(0, |results| results.query.page),
        pages: view.results.as_ref().map_or(0, |results| results.pages),
        found: view.results.as_ref().map_or(0, |results| results.found),
        sort: view.results.as_ref().and_then(|results| results.query.sort),
        sorts: view
            .results
            .as_ref()
            .map(|results| results.sorts.clone())
            .unwrap_or_default(),
        mine: my_listings(world)
            .iter()
            .map(|row| shown_listing(row, view, now))
            .collect(),
    })
}

/// The page of listings the player's latest search of the market found: none while the market is
/// shut, and none before the first search.
pub fn found(world: &World) -> Option<Vec<ShownListing>> {
    let view = view(world)?;
    let now = time::server_now(world)?;
    let results = view.results.as_ref()?;
    Some(
        results
            .rows
            .iter()
            .map(|row| shown_listing(row, view, now))
            .collect(),
    )
}

pub(crate) fn shown_listing(row: &ListingRow, view: &MarketView, now: UnixMillis) -> ShownListing {
    ShownListing {
        id: row.id,
        goods: shown_stacks(&row.goods),
        price: shown_stacks(&row.price),
        left: row.ends.since(now),
        mine: row.mine,
        gone: view.gone.contains(&row.id),
    }
}

pub(crate) fn view(world: &World) -> Option<&MarketView> {
    session::my_character(world)?.get::<MarketView>()
}

pub(crate) fn my_listings(world: &World) -> Vec<ListingRow> {
    session::my_character(world)
        .and_then(|me| me.get::<SocietyView>())
        .map(|view| view.listings.clone())
        .unwrap_or_default()
}

pub(crate) fn shown_stacks(stacks: &[ItemStack]) -> Vec<ShownMarketStack> {
    stacks
        .iter()
        .map(|stack| ShownMarketStack {
            item: stack.item,
            count: stack.count,
        })
        .collect()
}
