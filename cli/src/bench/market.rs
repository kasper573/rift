//! The market as a live server's players use it. The market is one for the whole server, so what
//! it costs grows with what players do there, not with the game's content: it holds a number of
//! listings for every player, read from the store as a server starting up reads it, and a share
//! of the players stand at the market, searching, listing and buying.

use bevy_ecs::prelude::*;
use engine::core::content::{Content, ContentScope};
use engine::core::store::{RecordChange, RecordRow, RecordStore, RecordStoreAddress};
use engine::core::time::UnixMillis;
use engine::core::units::Seconds;
use engine::systems::account::{AccountId, AccountIdentity};
use engine::systems::from_client;
use engine::systems::item::{Inventory, ItemDef, ItemId, ItemStack};
use engine::systems::market::{
    self, Listing, ListingId, MarketAsk, MarketQuery, MarketRequest, MarketSettings, MarketView,
};
use engine::systems::society::Society;
use engine::systems::{GameSettings, request_sources};

/// The listings the market holds for every player, as many as a seller may have up where the
/// game allows fewer.
pub const LISTINGS_PER_PLAYER: u32 = 10;

/// One player in this many stands at the market.
pub const TRADER_EVERY: usize = 20;

/// How many listings a searching player's window has room for.
const SEARCH_ROWS: u32 = 8;

/// Ticks between one trader's searches: three seconds at the server's rate.
const SEARCH_EVERY: usize = 90;

/// Ticks between one trader's listings, and between their purchases: thirty seconds.
const TRADE_EVERY: usize = 900;

/// What the traders asked of the market.
#[derive(Clone, Copy, Debug, Default)]
pub struct MarketAsks {
    pub searches: u64,
    pub listings: u64,
    pub purchases: u64,
}

/// Society as a server holding `accounts` starts it: the market holds `LISTINGS_PER_PLAYER`
/// listings of each, ending over the market's durations from `now`, read through a store as the
/// server reads it. A game without a market starts with none.
pub fn society(content: &Content, accounts: &[AccountIdentity], now: UnixMillis) -> Society {
    let Some(settings) = content.built::<GameSettings>().market.clone() else {
        return Society::new(request_sources());
    };
    let goods = goods(content, &settings);
    let scope = ContentScope::of(content);
    let per_seller = LISTINGS_PER_PLAYER.min(settings.max_listings);
    let rows: Vec<RecordChange> = accounts
        .iter()
        .flat_map(|account| (0..per_seller).map(move |listed| (account, listed)))
        .enumerate()
        .filter_map(|(index, (account, _))| {
            let listing = listing(content, &settings, &goods, &account.id, index, now)?;
            Some(RecordChange::Put(RecordRow::of(&listing, &scope)))
        })
        .collect();
    let folder = tempfile::tempdir().expect("a folder for the bench's store");
    let address = RecordStoreAddress::file(&folder.path().join("records.sqlite"));
    let runtime = tokio::runtime::Runtime::new().expect("a runtime to read the store on");
    runtime.block_on(async {
        let store = RecordStore::open(&address)
            .await
            .expect("the bench's store opens");
        store.write(rows).await.expect("the listings are written");
        Society::read(&store, content, request_sources())
            .await
            .expect("the listings are read back")
    })
}

/// A player of the bench who stands at the market.
pub struct Trader {
    pub conn: Entity,
    pub player: Entity,
    /// Which of the server's players they are, which spreads their turns over the ticks.
    pub number: usize,
}

/// Has `trader` do at the market what they do at `tick`: open it as they join, then search, list
/// and buy in turn.
pub fn trade(
    world: &mut World,
    content: &Content,
    trader: Trader,
    tick: usize,
    asks: &mut MarketAsks,
) {
    let Trader {
        conn,
        player,
        number,
    } = trader;
    let Some(settings) = content.built::<GameSettings>().market.clone() else {
        return;
    };
    let Some(view) = world.get::<MarketView>(player).cloned() else {
        market::open(world, player, None);
        return;
    };
    let turn = tick + number;
    let goods = goods(content, &settings);
    if turn.is_multiple_of(SEARCH_EVERY) && !goods.is_empty() {
        let query = MarketQuery {
            item: goods[(turn / SEARCH_EVERY) % goods.len()],
            sort: None,
            page: 0,
            rows: SEARCH_ROWS,
        };
        from_client(world, conn, MarketRequest::Ask(MarketAsk::Search(query)));
        asks.searches += 1;
    }
    if turn.is_multiple_of(TRADE_EVERY) {
        let row = view
            .results
            .iter()
            .flat_map(|results| &results.rows)
            .find(|row| !row.mine);
        if let Some(row) = row {
            let buy = MarketAsk::Buy {
                listing: row.id,
                offer: row.offer.clone(),
                price: row.price.clone(),
            };
            from_client(world, conn, MarketRequest::Ask(buy));
            asks.purchases += 1;
        }
    }
    if (turn + TRADE_EVERY / 2).is_multiple_of(TRADE_EVERY) {
        let sold = world.get::<Inventory>(player).and_then(|bag| {
            bag.slots
                .iter()
                .find(|stack| stack.count > 0 && goods.contains(&stack.item))
                .map(|stack| ItemStack::new(stack.item, 1))
        });
        if let Some(stack) = sold {
            let list = MarketAsk::List {
                offer: vec![stack],
                price: vec![ItemStack::new(settings.default_currency, price_of(turn))],
                duration: settings.durations[0],
            };
            from_client(world, conn, MarketRequest::Ask(list));
            asks.listings += 1;
        }
    }
}

/// The `index`th listing of the market: a stack of one of the goods for some of the default
/// currency, its ends spread over the market's durations.
fn listing(
    content: &Content,
    settings: &MarketSettings,
    goods: &[ItemId],
    seller: &AccountId,
    index: usize,
    now: UnixMillis,
) -> Option<Listing> {
    let item = *goods.get(index % goods.len().max(1))?;
    let most = item.get(content).stack_max().max(1);
    let duration = settings.durations[index % settings.durations.len()];
    let share = (index % 10 + 1) as f32 / 10.0;
    Some(Listing {
        id: ListingId(index as u64 + 1),
        seller: seller.clone(),
        offer: vec![ItemStack::new(item, 1 + index as u32 % most)].into(),
        price: vec![ItemStack::new(settings.default_currency, price_of(index))].into(),
        deposit: vec![settings.deposit()].into(),
        ends: now.after(Seconds(duration.0 * share)),
    })
}

/// What the market trades: every item that can change hands, but the default currency.
fn goods(content: &Content, settings: &MarketSettings) -> Vec<ItemId> {
    content
        .ids::<ItemDef>()
        .filter(|&item| item != settings.default_currency && !item.get(content).bound())
        .collect()
}

fn price_of(index: usize) -> u32 {
    1 + (index * 7 % 100) as u32
}
