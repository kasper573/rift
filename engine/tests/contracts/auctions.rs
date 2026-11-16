use bevy_ecs::prelude::*;
use engine::core::units::Seconds;
use engine::systems::auction::{
    self, AuctionAsk, AuctionHouseView, AuctionQuery, AuctionRequest, AuctionRow,
};
use engine::systems::inbox::InboxView;
use engine::systems::item::{ItemDef, ItemStack};
use engine::systems::request_sources;
use serde_json::json;
use testkit::{Game, Map};

use crate::saves::{Saves, checkpoint};
use crate::server::Server;

#[test]
fn an_auction_holds_its_stack_until_the_first_buyer_pays_its_price() {
    let mut host = Host::new(Server::of(game(), request_sources()));
    let [wren, hale, marrow] = host.sign_in_all();
    host.give(wren, "gold", 100);
    host.give(wren, "sword", 1);
    host.give(wren, "potion", 5);
    host.give(wren, "map", 1);
    host.give(wren, "stone", 1);
    host.give(hale, "gold", 500);
    host.give(marrow, "gold", 500);

    host.list(1, "sword", 1, 200);
    assert_eq!(
        (
            host.count(wren, "sword"),
            host.count(wren, "gold"),
            host.mine(1).len()
        ),
        (0, 80, 1),
        "listing takes the stack and the deposit at once"
    );

    let refusals: [(&str, u32, u32, &str); 3] = [
        ("map", 1, 10, "Tattered Map is bound"),
        ("potion", 5, 0, "Set a price above zero."),
        ("potion", 5, 1000, "Needs 20 more Gold"),
    ];
    for (item, count, price, said) in refusals {
        host.list(1, item, count, price);
        assert_eq!(
            (
                host.errors(1).last().map(String::as_str),
                host.mine(1).len()
            ),
            (Some(said), 1),
            "{said}: nothing is listed"
        );
    }
    host.list(1, "potion", 5, 50);
    host.list(1, "stone", 1, 5);
    assert_eq!(
        (
            host.errors(1).last().map(String::as_str),
            host.mine(1).len(),
            host.count(wren, "stone")
        ),
        (Some("You already have 2 auctions running."), 2, 1),
        "a seller runs only so many auctions"
    );

    host.search(2, "sword");
    let found = host.results(2);
    assert_eq!(
        found
            .iter()
            .map(|row| (row.stack, row.price, row.mine))
            .collect::<Vec<_>>(),
        [(host.stack("sword", 1), 200, false)],
        "a search finds auctions by their item's name"
    );
    host.search(1, "");
    assert_eq!(
        host.results(1)
            .iter()
            .map(|row| row.mine)
            .collect::<Vec<_>>(),
        [true, true],
        "a seller finds their own auctions too"
    );

    let sword = found[0];
    let buy = AuctionAsk::Buy {
        auction: sword.id,
        stack: sword.stack,
        price: sword.price,
    };
    host.search(3, "sword");
    host.server.send(2, AuctionRequest::Ask(buy.clone()));
    host.server.send(3, AuctionRequest::Ask(buy));
    host.server.step();
    host.server.step();
    assert_eq!(
        (
            host.count(hale, "sword"),
            host.count(hale, "gold"),
            host.count(marrow, "gold"),
            host.errors(3).last().cloned(),
            host.view(3).gone
        ),
        (
            1,
            300,
            500,
            Some("Someone bought Iron Sword just before you.".to_owned()),
            vec![sword.id]
        ),
        "the first purchase settled gets the stack, and a later one pays nothing"
    );
    assert_eq!(
        (
            host.count(wren, "gold"),
            host.server.told(1).last().cloned()
        ),
        (
            75 + 200 - 20 + 20,
            Some("Sold 1 Iron Sword for 200 Gold".to_owned())
        ),
        "the seller gets the price less the house's cut, and the deposit back"
    );
}

#[test]
fn an_auction_that_does_not_sell_goes_back_to_its_seller_wherever_they_are() {
    let saves = Saves::in_a_file();
    let content = game().load();
    let mut host = Host::new(Server::saving(&content, &saves, request_sources()));
    let [wren, ..] = host.sign_in_all();
    host.give(wren, "gold", 100);
    host.give(wren, "sword", 1);
    host.give(wren, "potion", 5);
    host.list(1, "potion", 5, 50);
    host.list(1, "sword", 1, 100);
    let potions = host.mine(1)[0].id;
    host.ask(1, AuctionAsk::Cancel(potions));
    assert_eq!(
        (
            host.count(wren, "potion"),
            host.count(wren, "gold"),
            host.mine(1).len()
        ),
        (5, 85, 1),
        "cancelling gives the stack back and keeps the deposit"
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
    assert_eq!(next.results(2).len(), 1, "auctions outlive their server");
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
                "Iron Sword didn't sell".to_owned(),
                vec![next.stack("sword", 1)]
            ))
        ),
        "an auction whose time is up goes back to its seller, by message while they are away"
    );
}

/// A server whose village has the auction house, and what its players do with it.
struct Host {
    server: Server,
}

impl Host {
    fn new(mut server: Server) -> Host {
        for sim in [&mut server.village, &mut server.forest] {
            sim.accept::<AuctionRequest>()
                .add((auction::requests, auction::hold));
        }
        Host { server }
    }

    /// Wren, Hale and Marrow sign in, each with the house open.
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
        auction::open(&mut self.server.village.world, player, None);
    }

    fn ask(&mut self, client: u32, ask: AuctionAsk) {
        self.server.send(client, AuctionRequest::Ask(ask));
        self.server.step();
        self.server.step();
    }

    fn list(&mut self, client: u32, item: &str, count: u32, price: u32) {
        let stack = self.stack(item, count);
        let duration = Seconds(3600.0);
        self.ask(
            client,
            AuctionAsk::List {
                stack,
                price,
                duration,
            },
        );
    }

    fn search(&mut self, client: u32, name: &str) {
        let query = AuctionQuery {
            name: name.to_owned(),
            ..AuctionQuery::default()
        };
        self.ask(client, AuctionAsk::Search(query));
    }

    fn results_after_search(&mut self, client: u32, name: &str) -> usize {
        self.search(client, name);
        self.results(client).len()
    }

    fn view(&self, client: u32) -> AuctionHouseView {
        let character = self.server.village.player(client);
        self.server
            .village
            .world
            .get::<AuctionHouseView>(character)
            .cloned()
            .expect("the house open")
    }

    fn results(&self, client: u32) -> Vec<AuctionRow> {
        self.view(client)
            .results
            .map(|results| results.rows)
            .unwrap_or_default()
    }

    fn mine(&mut self, client: u32) -> Vec<AuctionRow> {
        self.server.view(client).auctions
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

/// A game whose auction house is paid in Gold, keeps a tenth of a price as a deposit and a tenth
/// as its cut, and runs two auctions a seller.
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
            settings["messages"] = json!({
                "inbox": 5, "attachments": 6, "title_length": 60, "text_length": 1000, "a_minute": 2
            });
            settings["auctions"] = json!({
                "currency": testkit::path("item", "gold"),
                "durations": [3600.0, 7200.0],
                "deposit": 10.0,
                "cut": 10.0,
                "listings": 2,
            });
        })
}
