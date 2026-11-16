use bevy_ecs::prelude::*;
use engine::core::content::Content;
use engine::core::math::Pos;
use engine::core::units::{MeterPos, Meters};
use engine::systems::area::{self, AreaDef};
use engine::systems::inbox::InboxView;
use engine::systems::item::{Inventory, ItemDef, ItemStack};
use engine::systems::movement::{self, position};
use engine::systems::player::{ClientId, Players};
use engine::systems::script::Place;
use engine::systems::social_request::RequestAnswer;
use engine::systems::trade::{self, TradeRequest, TradeView};
use engine::systems::{combat, request_sources};
use serde_json::json;
use testkit::{Game, Map};

use crate::server::Server;

const WREN_AT: Pos<Meters> = Pos::new(2.0, 4.0);
const HALE_AT: Pos<Meters> = Pos::new(14.0, 4.0);

#[test]
fn a_trade_swaps_both_offers_once_both_accept_the_same_deal() {
    let mut host = Host::new();
    let [wren, hale] = host.traders();
    host.give(1, "gold", 120);
    host.give(1, "map", 1);
    host.give(2, "sword", 1);
    host.give(2, "dagger", 1);

    host.send(1, TradeRequest::Ask { with: hale });
    for _ in 0..400 {
        if !host.server.cards(2).is_empty() {
            break;
        }
        host.server.step();
    }
    assert!(
        host.at(1).distance(host.at(2)) <= Meters(6.0),
        "the asker walks into reach before asking"
    );
    assert_eq!(
        host.server.cards(2),
        [("Wren wants to trade with you.".to_owned(), false)],
        "the player asked gets a card"
    );
    host.server.answer(2, RequestAnswer::Accept);
    host.run(2);
    let (dana, kim) = (host.view(1), host.view(2));
    assert_eq!(
        (dana.map(|view| view.with), kim.map(|view| view.with)),
        (Some("Hale".to_owned()), Some("Wren".to_owned())),
        "accepting opens the trade for both"
    );

    host.send(1, TradeRequest::Offer(host.stack("map", 1)));
    host.send(1, TradeRequest::Offer(host.stack("gold", 200)));
    assert_eq!(
        (host.mine(1), host.errors(1).last().cloned()),
        (vec![], Some("Tattered Map is bound".to_owned())),
        "a bound item can't be offered, nor more than the bag holds"
    );
    host.send(1, TradeRequest::Offer(host.stack("gold", 30)));
    let seen = host.view(2).expect("Hale's trade");
    assert_eq!(
        (
            seen.theirs,
            seen.changed.map(|changed| changed.added),
            host.server.village.count(wren, "gold")
        ),
        (
            vec![host.stack("gold", 30)],
            Some(vec![host.stack("gold", 30).item]),
            120
        ),
        "what one offers shows to the other, marked new, and stays in the bag until the trade"
    );

    let version = host.view(1).expect("a trade").version;
    host.send(1, TradeRequest::Accept(version));
    assert!(
        !host.view(1).expect("a trade").i_accept,
        "a change can't be accepted until its wait is over"
    );
    host.server.later(3.0);
    host.send(1, TradeRequest::Accept(version));
    assert!(host.view(1).expect("a trade").i_accept);

    host.send(2, TradeRequest::Offer(host.stack("sword", 1)));
    let changed = host.view(1).expect("a trade");
    assert_eq!(
        (
            changed.i_accept,
            host.server
                .village
                .told(1)
                .last()
                .map(|told| told.text.plain_words())
        ),
        (false, Some("Hale changed their offer.".to_owned())),
        "a change undoes every accept, and tells whoever accepted"
    );
    host.server.later(3.0);
    host.send(1, TradeRequest::Accept(version));
    assert!(
        !host.view(1).expect("a trade").i_accept,
        "an accept of a deal that changed since is refused"
    );

    let version = changed.version;
    host.send(2, TradeRequest::Offer(host.stack("dagger", 1)));
    host.send(2, TradeRequest::Withdraw(host.stack("dagger", 1)));
    let version_now = host.view(1).expect("a trade").version;
    assert_ne!(version, version_now, "taking back is a change too");
    host.server.later(3.0);
    host.send(2, TradeRequest::Accept(version_now));
    host.send(1, TradeRequest::Accept(version_now));
    let village = &host.server.village;
    assert_eq!(
        (
            village.count(wren, "gold"),
            village.count(wren, "sword"),
            village.count(hale, "gold"),
            village.count(hale, "sword"),
            village.count(hale, "dagger"),
        ),
        (90, 1, 30, 0, 1),
        "both accepting the same deal swaps both offers"
    );
    assert_eq!(
        (host.view(1), host.view(2)),
        (None, None),
        "and closes both windows"
    );
    assert_eq!(
        host.feed(1).last().cloned(),
        Some("You traded with Hale. You got 1 Iron Sword and gave 30 Gold.".to_owned())
    );

    host.fill_bag(1);
    host.open_trade();
    host.send(2, TradeRequest::Offer(host.stack("dagger", 1)));
    host.server.later(3.0);
    let version = host.view(1).expect("a trade").version;
    host.send(1, TradeRequest::Accept(version));
    host.send(2, TradeRequest::Accept(version));
    host.server.step();
    host.server.step();
    let inbox = host.inbox(1);
    assert_eq!(
        (
            host.server.village.count(wren, "dagger"),
            inbox
                .rows
                .first()
                .map(|row| (row.title.clone(), row.attached.clone())),
            host.feed(1).last().cloned(),
        ),
        (
            0,
            Some((
                "Items from your trade with Hale".to_owned(),
                vec![host.stack("dagger", 1)]
            )),
            Some("No room in your bag for 1 Dagger: sent to your messages.".to_owned())
        ),
        "what a full bag has no room for is sent as a message, and the player is told"
    );
}

#[test]
fn a_trade_ends_for_both_when_either_cancels_or_can_no_longer_trade() {
    type Ending = fn(&mut Host);
    let endings: [(&str, &[&str], &[&str], Ending); 6] = [
        ("cancelled", &["Hale cancelled the trade."], &[], |host| {
            host.send(2, TradeRequest::Cancel);
        }),
        (
            "apart",
            &["The trade with Hale ended: you moved too far apart."],
            &["The trade with Wren ended: you moved too far apart."],
            |host| host.put(2, WREN_AT),
        ),
        (
            "fell",
            &["The trade with Hale ended: Hale fell."],
            &["The trade with Wren ended: Hale fell."],
            |host| {
                let hale = host.character(2);
                combat::inflict(&mut host.server.village.world, hale, None, 1_000_000.0);
            },
        ),
        (
            "warped",
            &["The trade with Hale ended: Hale left."],
            &[],
            |host| {
                let hale = host.character(2);
                let forest = Place {
                    area: host.server.village.id::<AreaDef>("forest"),
                    at: Pos::new(2.0, 2.0),
                };
                area::warp::warp(&mut host.server.village.world, hale, forest)
                    .expect("a warp to the forest");
            },
        ),
        (
            "left the game",
            &["The trade with Hale ended: Hale left."],
            &[],
            |host| host.server.village.disconnect(2),
        ),
        (
            "offer gone",
            &["The trade with Hale ended: an offered item left Hale's bag."],
            &["The trade with Wren ended: an offered item left Hale's bag."],
            |host| {
                let hale = host.character(2);
                let sword = host.stack("sword", 1);
                let content = host.content();
                host.server
                    .village
                    .world
                    .get_mut::<Inventory>(hale)
                    .expect("a bag")
                    .exchange(&content, &[sword], &[])
                    .expect("the sword was in the bag");
            },
        ),
    ];
    for (case, wren_told, hale_told, end) in endings {
        let mut host = Host::new();
        let [wren, _] = host.traders();
        host.give(1, "gold", 50);
        host.give(2, "sword", 1);
        host.open_trade();
        host.send(1, TradeRequest::Offer(host.stack("gold", 50)));
        host.send(2, TradeRequest::Offer(host.stack("sword", 1)));
        let before = (host.errors(1).len(), host.errors(2).len());
        end(&mut host);
        host.run(2);
        assert_eq!(
            (
                host.view(1),
                host.view(2),
                host.server.village.count(wren, "gold")
            ),
            (None, None, 50),
            "{case}: the trade ends for both, and nothing moved"
        );
        assert_eq!(
            host.errors(1)[before.0..],
            *wren_told,
            "{case}: Wren is told"
        );
        if !hale_told.is_empty() {
            assert_eq!(
                host.errors(2)[before.1..],
                *hale_told,
                "{case}: Hale is told"
            );
        }
    }

    let mut host = Host::new();
    host.traders();
    let marrow = host.server.sign_in(3, "Marrow", "village");
    host.put_at(marrow, Pos::new(5.0, 4.0));
    host.open_trade();
    host.send(
        3,
        TradeRequest::Ask {
            with: host.character(2),
        },
    );
    assert_eq!(
        host.errors(3).last().cloned(),
        Some("Hale is busy trading.".to_owned()),
        "a player who trades can't be asked by another"
    );
}

/// A server whose village has room to walk, and two players far apart in it.
struct Host {
    server: Server,
}

impl Host {
    fn new() -> Host {
        let mut server = Server::of(game(), request_sources());
        for sim in [&mut server.village, &mut server.forest] {
            sim.accept::<TradeRequest>()
                .add((trade::requests, trade::hold))
                .plan(movement::AdvanceWalks::default())
                .add(trade::approach);
            sim.world.init_resource::<trade::Trades>();
        }
        Host { server }
    }

    /// Wren and Hale sign in, apart in the village.
    fn traders(&mut self) -> [Entity; 2] {
        let wren = self.server.sign_in(1, "Wren", "village");
        let hale = self.server.sign_in(2, "Hale", "village");
        self.put_at(wren, WREN_AT);
        self.put_at(hale, HALE_AT);
        self.server.step();
        [wren, hale]
    }

    /// Wren asks Hale to trade from beside them, and Hale accepts.
    fn open_trade(&mut self) {
        self.put(1, Pos::new(HALE_AT.x - 2.0, HALE_AT.y));
        let hale = self.character(2);
        self.send(1, TradeRequest::Ask { with: hale });
        self.run(3);
        self.server.answer(2, RequestAnswer::Accept);
        self.run(2);
        assert!(self.view(1).is_some(), "a trade is open");
    }

    fn run(&mut self, steps: u32) {
        for _ in 0..steps {
            self.server.step();
        }
    }

    fn send<M: Message>(&mut self, client: u32, message: M) {
        self.server.send(client, message);
        self.server.step();
    }

    fn character(&self, client: u32) -> Entity {
        self.server.village.player(client)
    }

    fn put(&mut self, client: u32, at: Pos<Meters>) {
        let character = self.character(client);
        self.put_at(character, at);
    }

    fn put_at(&mut self, character: Entity, at: Pos<Meters>) {
        movement::relocate(&mut self.server.village.world, character, at);
    }

    fn at(&self, client: u32) -> Pos<Meters> {
        position(&self.server.village.world, self.character(client)).expect("a place")
    }

    fn give(&mut self, client: u32, item: &str, count: u32) {
        let character = self.character(client);
        self.server.village.give(character, item, count);
    }

    /// Fills every free slot of `client`'s bag with stones.
    fn fill_bag(&mut self, client: u32) {
        let character = self.character(client);
        let bag = self
            .server
            .village
            .world
            .get::<Inventory>(character)
            .expect("a bag");
        let free = bag.max - bag.slots.len() as u32;
        for _ in 0..free {
            self.server.village.give(character, "stone", 1);
        }
    }

    fn stack(&self, item: &str, count: u32) -> ItemStack {
        ItemStack::new(self.server.village.id::<ItemDef>(item), count)
    }

    fn content(&self) -> Content {
        self.server.village.world.resource::<Content>().clone()
    }

    /// The trade the player of `client` sees, while they play in the village.
    fn view(&self, client: u32) -> Option<TradeView> {
        let world = &self.server.village.world;
        let character = *world.resource::<Players>().0.get(&ClientId(client))?;
        world.get::<TradeView>(character).cloned()
    }

    fn mine(&self, client: u32) -> Vec<ItemStack> {
        self.view(client).map(|view| view.mine).unwrap_or_default()
    }

    fn errors(&mut self, client: u32) -> Vec<String> {
        self.server.village.errors(client)
    }

    fn feed(&mut self, client: u32) -> Vec<String> {
        self.server.told(client)
    }

    fn inbox(&self, client: u32) -> InboxView {
        let character = self.character(client);
        self.server
            .village
            .world
            .get::<InboxView>(character)
            .cloned()
            .unwrap_or_default()
    }
}

/// A game whose players trade within 6 m, two different items each, and whose bags hold four
/// stacks.
fn game() -> Game {
    let single = json!({ "kind": { "type": "resource" } });
    Game::new()
        .area("village", &Map::open(18, 9), json!([]))
        .area("forest", &Map::open(9, 9), json!([]))
        .hero("village", 2.0, 4.0)
        .sfx("ui_refuse")
        .vfx("damage_taken")
        .shader("damage_pop")
        .vfx("defeated")
        .item(
            "gold",
            json!({ "name": "Gold", "kind": { "type": "resource" }, "modules": [{ "type": "stack", "max": 1000 }] }),
        )
        .item("sword", single.clone())
        .item("dagger", single.clone())
        .item("stone", single)
        .item(
            "map",
            json!({ "kind": { "type": "resource" }, "modules": [{ "type": "bound" }] }),
        )
        .edit("item", "sword", |sword| sword["name"] = json!("Iron Sword"))
        .edit("item", "dagger", |dagger| dagger["name"] = json!("Dagger"))
        .edit("item", "map", |map| map["name"] = json!("Tattered Map"))
        .edit_settings(|settings| {
            settings["lingers_for"] = json!(30.0);
            settings["slots"]["bag"] = json!(4);
            settings["messages"] = json!({
                "inbox": 5, "attachments": 6, "title_length": 60, "text_length": 1000, "a_minute": 2
            });
            settings["trade"] = json!({ "reach": 6.0, "slots": 2, "accept_waits": 3.0 });
        })
}
