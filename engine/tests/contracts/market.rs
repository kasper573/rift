use bevy_ecs::prelude::*;
use engine::core::units::Seconds;
use engine::systems::item::{ItemDef, ItemId, ItemStack};
use engine::systems::mail::InboxView;
use engine::systems::market::{
    self, ListingRow, ListingSort, MarketAsk, MarketQuery, MarketRequest, MarketView, SortBy,
};
use engine::systems::{request_sources, window};
use serde_json::json;
use testkit::{Game, Map};

use crate::saves::{Saves, checkpoint};
use crate::server::Server;

#[test]
fn a_listing_holds_its_goods_until_the_first_buyer_pays_its_price() {
    let mut host = Host::new(Server::of(game(), request_sources()));
    let [wren, hale, marrow] = host.sign_in_all();
    host.give(wren, "gold", 30);
    host.give(wren, "sword", 1);
    host.give(wren, "potion", 5);
    host.give(wren, "map", 1);
    host.give(wren, "stone", 1);
    for buyer in [hale, marrow] {
        host.give(buyer, "gold", 500);
        host.give(buyer, "herb", 3);
    }

    let bundle: Stacks = &[("sword", 1), ("potion", 2)];
    let price: Stacks = &[("gold", 200), ("herb", 3)];
    host.list(1, bundle, price);
    assert_eq!(
        (
            host.count(wren, "sword"),
            host.count(wren, "potion"),
            host.count(wren, "gold"),
            host.mine(1).len()
        ),
        (0, 3, 10, 1),
        "listing takes what it offers and the flat deposit at once"
    );

    let refusals: [(Stacks, Stacks, &str); 9] = [
        (&[], &[("gold", 10)], "Offer something."),
        (&[("potion", 1)], &[], "Set a price."),
        (&[("map", 1)], &[("gold", 10)], "Tattered Map is bound."),
        (
            &[("potion", 1), ("stone", 1), ("gold", 1)],
            &[("herb", 1)],
            "An offer has at most 2 items.",
        ),
        (&[("potion", 1)], &[("gold", 0)], "Gold needs an amount."),
        (
            &[("potion", 1)],
            &[("gold", 1), ("gold", 2)],
            "Name each item once.",
        ),
        (
            &[("potion", 1)],
            &[("gold", 1), ("herb", 1), ("stone", 1)],
            "A price has at most 2 items.",
        ),
        (&[("potion", 1)], &[("map", 1)], "Tattered Map is bound."),
        (&[("potion", 3)], &[("gold", 50)], "Needs 10 Gold more"),
    ];
    for (goods, price, said) in refusals {
        host.list(1, goods, price);
        assert_eq!(
            (
                host.errors(1).last().map(String::as_str),
                host.mine(1).len()
            ),
            (Some(said), 1),
            "{said}: nothing is listed"
        );
    }
    host.give(wren, "gold", 10);
    host.list(1, &[("potion", 3)], &[("gold", 50)]);
    host.list(1, &[("stone", 1)], &[("gold", 5)]);
    assert_eq!(
        (
            host.errors(1).last().map(String::as_str),
            host.mine(1).len(),
            host.count(wren, "stone")
        ),
        (Some("You already have 2 listings up."), 2, 1),
        "a seller has only so many listings up"
    );

    host.search(2, "sword");
    let found = host.results(2);
    assert_eq!(
        host.shown(&found),
        [(host.stacks(bundle), host.stacks(price), false)],
        "a search finds the listings that offer its item"
    );
    host.search(1, "potion");
    assert_eq!(
        host.shown(&host.results(1)),
        [
            (
                host.stacks(&[("potion", 3)]),
                host.stacks(&[("gold", 50)]),
                true
            ),
            (host.stacks(bundle), host.stacks(price), true),
        ],
        "a listing shows under each item it offers, and a seller finds their own listings too"
    );

    let listing = found[0].clone();
    let buy = MarketAsk::Buy {
        listing: listing.id,
        goods: listing.goods,
        price: listing.price,
    };
    host.search(3, "sword");
    host.server.send(2, MarketRequest::Ask(buy.clone()));
    host.server.send(3, MarketRequest::Ask(buy));
    host.server.step();
    host.server.step();
    assert_eq!(
        (
            host.count(hale, "sword"),
            host.count(hale, "potion"),
            host.count(hale, "gold"),
            host.count(hale, "herb"),
            host.count(marrow, "gold"),
            host.errors(3).last().cloned(),
            host.view(3).gone
        ),
        (
            1,
            2,
            300,
            0,
            500,
            Some(
                "Someone bought the listing of 1 Iron Sword and 2 Potion just before you."
                    .to_owned()
            ),
            vec![listing.id]
        ),
        "the first purchase settled gets what the listing offers, and a later one gives nothing"
    );
    assert_eq!(
        (
            host.count(wren, "gold"),
            host.count(wren, "herb"),
            host.server.told(1).last().cloned()
        ),
        (
            200 + 20,
            3,
            Some(
                "Your listing of 1 Iron Sword and 2 Potion was bought. You got 200 Gold and 3 Herb."
                    .to_owned()
            )
        ),
        "the seller gets the whole price, and the deposit back"
    );

    host.give(marrow, "potion", 5);
    host.list(2, &[("potion", 2)], &[("gold", 30)]);
    host.list(3, &[("potion", 4)], &[("gold", 40)]);
    host.list(3, &[("potion", 1)], &[("gold", 1), ("herb", 1)]);
    let deals = |host: &mut Host, sort: Option<(Side, &str, bool)>| {
        host.sorted(1, "potion", sort)
            .into_iter()
            .map(|row| (row.goods, row.price))
            .collect::<Vec<_>>()
    };
    let (four, two, three, one) = (
        (host.stacks(&[("potion", 4)]), host.stacks(&[("gold", 40)])),
        (host.stacks(&[("potion", 2)]), host.stacks(&[("gold", 30)])),
        (host.stacks(&[("potion", 3)]), host.stacks(&[("gold", 50)])),
        (
            host.stacks(&[("potion", 1)]),
            host.stacks(&[("gold", 1), ("herb", 1)]),
        ),
    );
    assert_eq!(
        deals(&mut host, Some((SortBy::Price, "gold", false))),
        [one.clone(), four.clone(), two.clone(), three.clone()],
        "a search sorts by each listing's price in the chosen item for one of the item searched \
         for, the lowest first"
    );
    assert_eq!(
        deals(&mut host, None),
        [one.clone(), four.clone(), two.clone(), three.clone()],
        "a search that asks for no sort sorts by the item in the most listings' prices, the lowest first"
    );
    assert_eq!(
        deals(&mut host, Some((SortBy::Price, "gold", true))),
        [three.clone(), two.clone(), four.clone(), one.clone()],
        "or the most first"
    );
    assert_eq!(
        (
            deals(&mut host, Some((SortBy::Goods, "potion", false))),
            deals(&mut host, Some((SortBy::Goods, "potion", true))),
        ),
        (
            vec![one.clone(), two.clone(), three.clone(), four.clone()],
            vec![four.clone(), three.clone(), two.clone(), one.clone()],
        ),
        "a search sorts by how many of an item each listing offers too, either way"
    );
    assert_eq!(
        (
            deals(&mut host, Some((SortBy::Price, "herb", false))),
            host.view(1).results.map(|results| results.sorts)
        ),
        (
            vec![one, three, two, four],
            Some(vec![
                SortBy::Goods(host.stack("potion", 1).item),
                SortBy::Price(host.stack("gold", 1).item),
                SortBy::Price(host.stack("herb", 1).item),
            ])
        ),
        "listings whose price lacks the item sorted by come after, those ending first first, and a \
         search can be sorted by any item the listings it found offer or price in"
    );
}

#[test]
fn a_listing_nobody_buys_goes_back_to_its_seller_wherever_they_are() {
    let saves = Saves::in_a_file();
    let content = game().load();
    let mut host = Host::new(Server::saving(&content, &saves, request_sources()));
    let [wren, ..] = host.sign_in_all();
    host.give(wren, "gold", 100);
    host.give(wren, "sword", 1);
    host.give(wren, "potion", 5);
    host.list(1, &[("potion", 5)], &[("gold", 50)]);
    host.list(1, &[("sword", 1)], &[("gold", 100)]);
    let potions = host.mine(1)[0].id;
    host.ask(1, MarketAsk::Cancel(potions));
    assert_eq!(
        (
            host.count(wren, "potion"),
            host.count(wren, "gold"),
            host.mine(1).len()
        ),
        (5, 60, 1),
        "cancelling gives back what the listing offers and keeps the deposit"
    );
    let report = checkpoint(
        &mut [&mut host.server.village, &mut host.server.forest],
        &mut host.server.above,
    );
    assert!(report.outcome.is_ok());

    let mut next = Host::new(Server::saving(&content, &saves, request_sources()));
    let hale = next.server.sign_in(2, "Hale", "village");
    next.open(hale);
    next.search(2, "sword");
    assert_eq!(next.results(2).len(), 1, "listings outlive their server");
    next.server.later(3700.0);
    next.server.sign_in(1, "Wren", "village");
    next.server.step();
    next.server.step();
    let inbox = next.inbox(1);
    assert_eq!(
        (
            next.results_after_search(2, "sword"),
            inbox
                .rows
                .first()
                .map(|row| (row.title.clone(), row.attached.clone()))
        ),
        (
            0,
            Some((
                "Back from the market".to_owned(),
                vec![next.stack("sword", 1)]
            ))
        ),
        "a listing whose time is up goes back to its seller, by message while they are away"
    );
}

/// Stacks as a test names them: each item by its name, with how many of it.
type Stacks<'a> = &'a [(&'a str, u32)];

/// The side of a listing a test sorts by: its goods or its price.
type Side = fn(ItemId) -> SortBy;

/// A server whose village has the market, and what its players do with it.
struct Host {
    server: Server,
}

impl Host {
    fn new(mut server: Server) -> Host {
        for sim in [&mut server.village, &mut server.forest] {
            sim.accept::<MarketRequest>()
                .add((market::requests, window::hold::<market::MarketVisit>));
        }
        Host { server }
    }

    /// Wren, Hale and Marrow sign in, each with the market open.
    fn sign_in_all(&mut self) -> [Entity; 3] {
        let players = [(1, "Wren"), (2, "Hale"), (3, "Marrow")]
            .map(|(client, name)| self.server.sign_in(client, name, "village"));
        for player in players {
            self.open(player);
        }
        self.server.step();
        players
    }

    fn open(&mut self, player: Entity) {
        market::open(&mut self.server.village.world, player, None);
    }

    fn ask(&mut self, client: u32, ask: MarketAsk) {
        self.server.send(client, MarketRequest::Ask(ask));
        self.server.step();
        self.server.step();
    }

    fn list(&mut self, client: u32, goods: Stacks, price: Stacks) {
        let ask = MarketAsk::List {
            goods: self.stacks(goods),
            price: self.stacks(price),
            duration: Seconds(3600.0),
        };
        self.ask(client, ask);
    }

    fn search(&mut self, client: u32, item: &str) {
        self.sorted(client, item, None);
    }

    /// What a search for `item` finds sorted by the side and item `sort` names, the most first
    /// where it says so.
    fn sorted(
        &mut self,
        client: u32,
        item: &str,
        sort: Option<(Side, &str, bool)>,
    ) -> Vec<ListingRow> {
        let query = MarketQuery {
            item: self.stack(item, 1).item,
            sort: sort.map(|(side, by, descending)| ListingSort {
                by: side(self.stack(by, 1).item),
                descending,
            }),
            page: 0,
            rows: 50,
        };
        self.ask(client, MarketAsk::Search(query));
        self.results(client)
    }

    fn results_after_search(&mut self, client: u32, item: &str) -> usize {
        self.search(client, item);
        self.results(client).len()
    }

    fn view(&self, client: u32) -> MarketView {
        let character = self.server.village.player(client);
        self.server
            .village
            .world
            .get::<MarketView>(character)
            .cloned()
            .expect("the market open")
    }

    fn results(&self, client: u32) -> Vec<ListingRow> {
        self.view(client)
            .results
            .map(|results| results.rows)
            .unwrap_or_default()
    }

    fn mine(&mut self, client: u32) -> Vec<ListingRow> {
        self.server.view(client).listings
    }

    /// Each of `rows`' goods and price, and whether the player shown it listed it.
    fn shown(&self, rows: &[ListingRow]) -> Vec<(Vec<ItemStack>, Vec<ItemStack>, bool)> {
        rows.iter()
            .map(|row| (row.goods.clone(), row.price.clone(), row.mine))
            .collect()
    }

    fn give(&mut self, player: Entity, item: &str, count: u32) {
        self.server.village.give(player, item, count);
    }

    fn count(&self, player: Entity, item: &str) -> u32 {
        self.server.village.count(player, item)
    }

    fn stack(&self, item: &str, count: u32) -> ItemStack {
        ItemStack::new(self.server.village.id::<ItemDef>(item), count)
    }

    fn stacks(&self, stacks: Stacks) -> Vec<ItemStack> {
        stacks
            .iter()
            .map(|(item, count)| self.stack(item, *count))
            .collect()
    }

    fn errors(&mut self, client: u32) -> Vec<String> {
        self.server.village.errors(client)
    }

    /// The inbox of the player of `client`, once the server has all of it in hand.
    fn inbox(&mut self, client: u32) -> InboxView {
        for _ in 0..5000 {
            self.server.step();
            let character = self.server.village.player(client);
            let inbox = self.server.village.world.get::<InboxView>(character);
            if let Some(inbox) = inbox.filter(|inbox| inbox.whole) {
                return inbox.clone();
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        panic!("the inbox of client {client} was never read");
    }
}

/// A game whose market starts a listing's price on Herb, takes listings of up to two items a
/// side, keeps 20 Gold as a deposit, and lets a seller have two listings up.
fn game() -> Game {
    let single = json!({ "kind": { "type": "resource" } });
    Game::new()
        .area("village", &Map::open(9, 9), json!([]))
        .area("forest", &Map::open(9, 9), json!([]))
        .hero("village", 4.0, 4.0)
        .sfx("ui_refuse")
        .item(
            "gold",
            json!({ "name": "Gold", "kind": { "type": "resource" }, "modules": [{ "type": "stack", "max": 10000 }] }),
        )
        .item(
            "potion",
            json!({ "name": "Potion", "kind": { "type": "resource" }, "modules": [{ "type": "stack", "max": 20 }] }),
        )
        .item(
            "herb",
            json!({ "name": "Herb", "kind": { "type": "resource" }, "modules": [{ "type": "stack", "max": 20 }] }),
        )
        .item("sword", single.clone())
        .item("stone", single)
        .item(
            "map",
            json!({ "kind": { "type": "resource" }, "modules": [{ "type": "bound" }] }),
        )
        .edit("item", "sword", |sword| sword["name"] = json!("Iron Sword"))
        .edit("item", "map", |map| map["name"] = json!("Tattered Map"))
        .edit_settings(|settings| {
            settings["lingers_for"] = json!(0.0);
            settings["mail"] = json!({
                "inbox": 5, "attachments": 6, "title_length": 60, "text_length": 1000, "a_minute": 2
            });
            settings["market"] = json!({
                "default_currency": testkit::path("item", "herb"),
                "deposit_currency": testkit::path("item", "gold"),
                "max_goods_items": 2,
                "max_price_items": 2,
                "durations": [3600.0, 7200.0],
                "deposit": 20,
                "max_listings": 2,
                "page_rows": 50,
            });
        })
}
