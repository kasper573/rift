//! The auction house: one market for the whole server. A seller lists a stack at a fixed price
//! for one of the game's lengths, and the first player to pay that price gets it. The house holds
//! the stack from the moment it is listed, so every way an auction ends pays out what it holds,
//! in the same settle that ends it.

pub mod effect;
pub mod window;

use std::collections::{BTreeMap, BTreeSet};

use bevy_app::App;
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::{Content, ContentProblem, Loading, SETTINGS_FILE, SavedList};
use crate::core::store::{Record, RecordId};
use crate::core::time::UnixMillis;
use crate::core::units::{MeterPos, Meters, Seconds};
use crate::systems::account::AccountId;
use crate::systems::area::transition::Crossing;
use crate::systems::history::HistoryTopic;
use crate::systems::inbox;
use crate::systems::interact::Interactive;
use crate::systems::item::rarity::ItemRarityId;
use crate::systems::item::{Inventory, ItemCategory, ItemId, ItemStack};
use crate::systems::mention::Mention;
use crate::systems::notification::Notification;
use crate::systems::payout::{self, PayoutReason};
use crate::systems::player::{self, PlayedBy, RequestNeeds};
use crate::systems::script::{self, EffectReach, OwnerRow, RiftScripts};
use crate::systems::society::{self, Settling, Society, SocietyAsk};
use crate::systems::text::LineText;
use crate::systems::{GameSettings, GameWords, area, movement, stat};

pub fn register(app: &mut App) {
    use bevy_replication::{AppMessageExt, AppReplicationExt};
    app.replicate_private::<AuctionHouseView>()
        .add_client_message::<AuctionRequest>();
}

/// How a game's auction house works. A game without it has none.
#[derive(JsonSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuctionSettings {
    /// The item every price is paid in.
    pub currency: ItemId,
    /// The lengths a seller may choose from.
    pub durations: Vec<Seconds>,
    /// Percent of the price a listing costs, kept by the house unless the stack sells.
    pub deposit: f32,
    /// Percent of the price the house keeps when the stack sells.
    pub cut: f32,
    /// How many auctions an account may have running.
    pub listings: u32,
}

impl AuctionSettings {
    /// What listing a stack at `price` costs.
    pub fn deposit_on(&self, price: u32) -> u32 {
        percent(price, self.deposit, f32::ceil)
    }

    /// What the house keeps of `price` when the stack sells.
    pub fn cut_of(&self, price: u32) -> u32 {
        percent(price, self.cut, f32::floor)
    }
}

#[derive(
    JsonSchema, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
pub struct AuctionId(pub u64);

/// A stack on sale.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Auction {
    pub id: AuctionId,
    pub seller: AccountId,
    /// The one stack listed, held by the house. It waits set aside while the game no longer has
    /// its item, and goes back to its seller as the auction ends.
    pub stack: SavedList<ItemStack>,
    /// What the whole stack costs, in the house's currency.
    pub price: u32,
    /// What listing it cost, given back if it sells.
    pub deposit: u32,
    pub ends: UnixMillis,
}

impl Record for Auction {
    const KIND: &'static str = "auction";

    fn id(&self) -> RecordId {
        RecordId(self.id.0.to_string())
    }
}

impl Auction {
    /// The stack on sale, while the game has its item.
    pub fn on_sale(&self) -> Option<ItemStack> {
        self.stack.first().copied()
    }
}

/// Every auction running on the server.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AuctionHouse {
    live: BTreeMap<AuctionId, Auction>,
    ends: BTreeSet<(UnixMillis, AuctionId)>,
    dealt: u64,
}

impl AuctionHouse {
    pub fn of(saved: impl IntoIterator<Item = Auction>) -> AuctionHouse {
        let mut house = AuctionHouse::default();
        for auction in saved {
            house.dealt = house.dealt.max(auction.id.0);
            house.hold(auction);
        }
        house
    }

    pub fn get(&self, id: AuctionId) -> Option<&Auction> {
        self.live.get(&id)
    }

    /// The auctions `seller` has running, those ending first first.
    pub fn of_seller<'a>(&'a self, seller: &'a AccountId) -> impl Iterator<Item = &'a Auction> {
        self.ends
            .iter()
            .filter_map(|(_, id)| self.live.get(id))
            .filter(move |auction| auction.seller == *seller)
    }

    fn add(&mut self, auction: Auction) -> Auction {
        self.dealt += 1;
        let auction = Auction {
            id: AuctionId(self.dealt),
            ..auction
        };
        self.hold(auction.clone());
        auction
    }

    fn hold(&mut self, auction: Auction) {
        self.ends.insert((auction.ends, auction.id));
        self.live.insert(auction.id, auction);
    }

    fn remove(&mut self, id: AuctionId) -> Option<Auction> {
        let auction = self.live.remove(&id)?;
        self.ends.remove(&(auction.ends, id));
        Some(auction)
    }

    /// The first auction whose time is up at `now`.
    fn ended(&self, now: UnixMillis) -> Option<AuctionId> {
        self.ends
            .first()
            .filter(|(ends, _)| *ends <= now)
            .map(|&(_, id)| id)
    }
}

/// What a player asks of the house.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum AuctionAsk {
    Search(AuctionQuery),
    List {
        stack: ItemStack,
        price: u32,
        duration: Seconds,
    },
    /// Buys the auction, as the buyer saw it: a stack whose seller changed it since is not
    /// bought.
    Buy {
        auction: AuctionId,
        stack: ItemStack,
        price: u32,
    },
    Cancel(AuctionId),
}

/// What a player asks of the house window.
#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum AuctionRequest {
    Ask(AuctionAsk),
    Close,
}

/// A search of the house: auctions of items whose name holds `name`, of the category and rarity
/// asked for where one is, a page at a time.
#[derive(JsonSchema, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct AuctionQuery {
    pub name: String,
    pub category: Option<ItemCategory>,
    pub rarity: Option<ItemRarityId>,
    pub page: u32,
}

/// An auction as a player sees it.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct AuctionRow {
    pub id: AuctionId,
    pub stack: ItemStack,
    pub price: u32,
    pub ends: UnixMillis,
    /// Whether the player it is shown to listed it.
    pub mine: bool,
}

/// What a search found: a page of the auctions that match, those cheapest for each item first.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AuctionResults {
    pub query: AuctionQuery,
    pub rows: Vec<AuctionRow>,
    pub pages: u32,
}

/// The house as the player who has it open sees it.
#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct AuctionHouseView {
    /// Raised by every answer of the house, so a client can tell an answer from silence.
    pub answered: u32,
    /// What the latest search found: none before the first.
    pub results: Option<AuctionResults>,
    /// The auctions of those results someone else bought before this player's purchase settled.
    pub gone: Vec<AuctionId>,
}

/// How many auctions a page of results lists.
pub const PAGE_ROWS: u32 = 8;

/// Opens the house for `player`, from `opener` where a thing in the world opened it: then it
/// closes as they walk out of its reach.
pub fn open(world: &mut World, player: Entity, opener: Option<Entity>) {
    if world.resource::<GameSettings>().auctions.is_none() {
        return;
    }
    let opener = opener
        .filter(|opener| *opener != player)
        .and_then(|opener| Some((opener, world.get::<Interactive>(opener)?.reach)));
    world
        .entity_mut(player)
        .insert((AuctionVisit { opener }, AuctionHouseView::default()));
}

/// Hands what players of this world asked of the house to society.
pub fn requests(world: &mut World) {
    player::player_requests::<AuctionRequest>(
        world,
        RequestNeeds::Nothing,
        |world, player, request| {
            match request {
                AuctionRequest::Close => close(world, player),
                AuctionRequest::Ask(ask) => {
                    let client = world.get::<PlayedBy>(player).map(|by| by.client);
                    if let (Some(client), true) =
                        (client, world.get::<AuctionVisit>(player).is_some())
                    {
                        society::ask(world, client, SocietyAsk::Auction(ask));
                    }
                }
            }
            Ok(())
        },
    );
}

/// Closes the house of whoever fell, or walked out of reach of what opened it.
pub fn hold(world: &mut World, visits: &mut QueryState<(Entity, &'static AuctionVisit)>) {
    let over: Vec<Entity> = visits
        .iter(world)
        .filter(|(player, visit)| {
            stat::is_dead(world, *player)
                || world.get::<Crossing>(*player).is_some()
                || visit
                    .opener
                    .is_some_and(|(opener, reach)| !within(world, *player, opener, reach))
        })
        .map(|(player, _)| player)
        .collect();
    for player in over {
        close(world, player);
    }
}

/// Does what `by` asked of the house.
pub(crate) fn settle(settling: &mut Settling, content: &Content, by: &AccountId, ask: AuctionAsk) {
    let Some(settings) = content.built::<GameSettings>().auctions.clone() else {
        return;
    };
    match ask {
        AuctionAsk::Search(query) => search(settling, content, by, query),
        AuctionAsk::List {
            stack,
            price,
            duration,
        } => list(settling, content, &settings, by, stack, price, duration),
        AuctionAsk::Buy {
            auction,
            stack,
            price,
        } => buy(settling, content, &settings, by, auction, stack, price),
        AuctionAsk::Cancel(auction) => cancel(settling, content, by, auction),
    }
    show(settling, by, |view| {
        view.answered = view.answered.wrapping_add(1)
    });
}

/// Ends every auction whose time is up, its stack going back to its seller.
pub(crate) fn keep(settling: &mut Settling, content: &Content) {
    while let Some(id) = settling.society.auctions.ended(settling.now) {
        let Some(auction) = settling.society.auctions.remove(id) else {
            continue;
        };
        settling.unwritten.remove::<Auction>(auction.id());
        give_back(settling, content, auction, Returned::Expired);
    }
}

/// The auctions `account` has running, as they see them.
pub(crate) fn mine(society: &Society, account: &AccountId) -> Vec<AuctionRow> {
    society
        .auctions
        .of_seller(account)
        .filter_map(|auction| row(auction, account))
        .collect()
}

/// The words a game's settings must have for the house, checked as the game loads.
pub(crate) fn words(words: &GameWords) -> [(&'static str, &String); 9] {
    [
        ("auction_listed", &words.auction_listed),
        ("auction_bought", &words.auction_bought),
        ("auction_sold", &words.auction_sold),
        ("auction_expired", &words.auction_expired),
        ("auction_cancelled", &words.auction_cancelled),
        ("auction_gone", &words.auction_gone),
        ("auction_too_many", &words.auction_too_many),
        ("auction_message", &words.auction_message),
        ("auction_no_price", &words.auction_no_price),
    ]
}

/// A game whose content opens the auction house has one, paid in an item that can change hands.
pub(crate) fn check(loading: &mut Loading, content: &Content, settings: &GameSettings) {
    if let Some(auctions) = &settings.auctions
        && auctions.currency.get(content).bound()
    {
        let problem = ContentProblem::new(
            "bound currency",
            "a bound item",
            "an item that can change hands",
        );
        loading.json_error(SETTINGS_FILE, "/auctions/currency", problem);
    }
    if settings.auctions.is_some() {
        return;
    }
    let scripts = content.built::<RiftScripts>();
    let opening: Vec<OwnerRow> = scripts
        .owners()
        .map(|(row, _)| *row)
        .filter(|row| {
            scripts
                .may_do(*row)
                .reaches
                .contains(&EffectReach::OpensAuctionHouse)
        })
        .collect();
    for row in opening {
        let site = script::owner_site(content, row);
        let problem = ContentProblem::new(
            "no auction house",
            "a script that opens the auction house",
            "auctions in the game's settings",
        );
        loading.json_error(&site.file, &site.pointer, problem);
    }
}

/// Where a player has the house open from.
#[derive(Component)]
pub struct AuctionVisit {
    /// The thing that opened it, and how far from it the player may stand.
    opener: Option<(Entity, Meters)>,
}

/// Why an auction's stack goes back to its seller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Returned {
    Expired,
    Cancelled,
}

fn close(world: &mut World, player: Entity) {
    if let Ok(mut player) = world.get_entity_mut(player) {
        player.remove::<(AuctionVisit, AuctionHouseView)>();
    }
}

fn within(world: &World, player: Entity, opener: Entity, reach: Meters) -> bool {
    let near = match (
        movement::position(world, player),
        movement::position(world, opener),
    ) {
        (Some(player), Some(opener)) => player.distance(opener) <= reach,
        _ => false,
    };
    near && area::same_area(world, player, opener)
}

fn search(settling: &mut Settling, content: &Content, by: &AccountId, query: AuctionQuery) {
    let name = query.name.trim().to_lowercase();
    let mut found: Vec<AuctionRow> = settling
        .society
        .auctions
        .live
        .values()
        .filter_map(|auction| row(auction, by))
        .filter(|row| {
            let def = row.stack.item.get(content);
            def.name.to_lowercase().contains(&name)
                && query
                    .category
                    .is_none_or(|category| def.category() == category)
                && query.rarity.is_none_or(|rarity| def.rarity == rarity)
        })
        .collect();
    found.sort_by(|one, other| {
        let name = |row: &AuctionRow| row.stack.item.get(content).name.clone();
        let each = |row: &AuctionRow| f64::from(row.price) / f64::from(row.stack.count.max(1));
        name(one)
            .cmp(&name(other))
            .then(each(one).total_cmp(&each(other)))
            .then(one.ends.cmp(&other.ends))
    });
    let pages = (found.len() as u32).div_ceil(PAGE_ROWS).max(1);
    let page = query.page.min(pages - 1);
    let rows = found
        .into_iter()
        .skip((page * PAGE_ROWS) as usize)
        .take(PAGE_ROWS as usize)
        .collect();
    let results = AuctionResults {
        query: AuctionQuery { page, ..query },
        rows,
        pages,
    };
    show(settling, by, |view| {
        view.results = Some(results);
        view.gone.clear();
    });
}

fn list(
    settling: &mut Settling,
    content: &Content,
    settings: &AuctionSettings,
    by: &AccountId,
    stack: ItemStack,
    price: u32,
    duration: Seconds,
) {
    let words = &content.built::<GameSettings>().words;
    let item = Mention::Item(stack.item).line(content);
    if price == 0 {
        return settling.refuse(by, words.auction_no_price.clone());
    }
    if stack.count == 0 || !settings.durations.contains(&duration) {
        return;
    }
    if stack.item.get(content).bound() {
        return settling.refuse(by, item.then(" is bound").plain_words());
    }
    let running = settling.society.auctions.of_seller(by).count() as u32;
    if running >= settings.listings {
        let full = words
            .auction_too_many
            .replace("{listings}", &settings.listings.to_string());
        return settling.refuse(by, full);
    }
    let deposit = settings.deposit_on(price);
    let costs = [stack, ItemStack::new(settings.currency, deposit)];
    let costs: Vec<ItemStack> = costs.into_iter().filter(|cost| cost.count > 0).collect();
    let Some((world, character)) = settling.reach(by) else {
        return;
    };
    let taken = world
        .get_mut::<Inventory>(character)
        .map(|mut bag| bag.exchange(content, &costs, &[]));
    match taken {
        Some(Ok(())) => {}
        Some(Err(refusal)) => return settling.refuse(by, refusal.describe(content)),
        None => return,
    }
    let auction = settling.society.auctions.add(Auction {
        id: AuctionId(0),
        seller: by.clone(),
        stack: vec![stack].into(),
        price,
        deposit,
        ends: settling.now.after(duration),
    });
    settling.unwritten.put(auction);
    settling.society.changed(by);
    let said =
        LineText::plain(words.auction_listed.as_str()).filling("{item}", stack.named(content));
    settling.tell(by, Notification::feed(HistoryTopic::Item, said));
}

fn buy(
    settling: &mut Settling,
    content: &Content,
    settings: &AuctionSettings,
    by: &AccountId,
    id: AuctionId,
    seen: ItemStack,
    price: u32,
) {
    let words = &content.built::<GameSettings>().words;
    let unchanged = settling.society.auctions.get(id).is_some_and(|auction| {
        auction.on_sale() == Some(seen) && auction.price == price && auction.seller != *by
    });
    if !unchanged {
        let gone = settling.society.auctions.get(id).is_none();
        if gone {
            let said = LineText::plain(words.auction_gone.as_str())
                .filling("{item}", Mention::Item(seen.item).span(content));
            settling.refuse(by, said.plain_words());
            show(settling, by, |view| view.gone.push(id));
        }
        return;
    }
    let paid = ItemStack::new(settings.currency, price);
    let Some((world, character)) = settling.reach(by) else {
        return;
    };
    let taken = world
        .get_mut::<Inventory>(character)
        .map(|mut bag| bag.exchange(content, &[paid], &[]));
    match taken {
        Some(Ok(())) => {}
        Some(Err(refusal)) => return settling.refuse(by, refusal.describe(content)),
        None => return,
    }
    let Some(auction) = settling.society.auctions.remove(id) else {
        return;
    };
    settling.unwritten.remove::<Auction>(auction.id());
    let item = seen.item;
    let bought = payout::pay_account(
        settling,
        content,
        by,
        &[seen],
        PayoutReason::AuctionBought(item),
    );
    let said = LineText::plain(words.auction_bought.as_str())
        .filling("{item}", seen.named(content))
        .filling("{price}", paid.named(content));
    settling.tell(by, Notification::feed(HistoryTopic::Item, said));
    if let Some(overflow) = bought.overflow(content) {
        settling.tell(by, Notification::feed(HistoryTopic::Item, overflow));
    }
    let earned = price - settings.cut_of(price) + auction.deposit;
    let earned = ItemStack::new(settings.currency, earned);
    payout::pay_account(
        settling,
        content,
        &auction.seller,
        &[earned],
        PayoutReason::AuctionSold(item),
    );
    let said = LineText::plain(words.auction_sold.as_str())
        .filling("{item}", seen.named(content))
        .filling("{price}", earned.named(content));
    settling.tell(
        &auction.seller,
        Notification::feed(HistoryTopic::Item, said),
    );
    settling.society.changed(by);
    settling.society.changed(&auction.seller);
}

fn cancel(settling: &mut Settling, content: &Content, by: &AccountId, id: AuctionId) {
    let own = settling
        .society
        .auctions
        .get(id)
        .is_some_and(|auction| auction.seller == *by);
    if !own {
        return;
    }
    let Some(auction) = settling.society.auctions.remove(id) else {
        return;
    };
    settling.unwritten.remove::<Auction>(auction.id());
    give_back(settling, content, auction, Returned::Cancelled);
}

/// Pays the stack of an auction that did not sell back to its seller, wherever they are: one
/// set aside goes by message, as it was saved.
fn give_back(settling: &mut Settling, content: &Content, auction: Auction, why: Returned) {
    let words = &content.built::<GameSettings>().words;
    let seller = auction.seller.clone();
    settling.society.changed(&seller);
    if let Some(stack) = auction.on_sale() {
        let reason = match why {
            Returned::Expired => PayoutReason::AuctionExpired(stack.item),
            Returned::Cancelled => PayoutReason::AuctionCancelled(stack.item),
        };
        let paid = payout::pay_account(settling, content, &seller, &[stack], reason);
        let said = match why {
            Returned::Expired => &words.auction_expired,
            Returned::Cancelled => &words.auction_cancelled,
        };
        let said = LineText::plain(said.as_str()).filling("{item}", stack.named(content));
        settling.tell(&seller, Notification::feed(HistoryTopic::Item, said));
        if let Some(overflow) = paid.overflow(content) {
            settling.tell(&seller, Notification::feed(HistoryTopic::Item, overflow));
        }
        return;
    }
    let named = auction
        .stack
        .set_aside()
        .iter()
        .flat_map(|aside| &aside.missing)
        .map(|missing| missing.path.clone())
        .next()
        .unwrap_or_default();
    let title = words.auction_expired.replace("{item}", &named);
    inbox::from_game(
        settling.society,
        settling.unwritten,
        settling.now,
        &seller,
        (&title, &words.auction_message),
        auction.stack,
    );
}

/// Changes `by`'s window as `edit` says, while they have the house open.
fn show(settling: &mut Settling, by: &AccountId, edit: impl FnOnce(&mut AuctionHouseView)) {
    let Some((world, character)) = settling.reach(by) else {
        return;
    };
    if let Some(mut view) = world.get_mut::<AuctionHouseView>(character) {
        edit(&mut view);
    }
}

fn row(auction: &Auction, viewer: &AccountId) -> Option<AuctionRow> {
    Some(AuctionRow {
        id: auction.id,
        stack: auction.on_sale()?,
        price: auction.price,
        ends: auction.ends,
        mine: auction.seller == *viewer,
    })
}

fn percent(price: u32, percent: f32, round: fn(f32) -> f32) -> u32 {
    round(price as f32 * percent / 100.0) as u32
}
