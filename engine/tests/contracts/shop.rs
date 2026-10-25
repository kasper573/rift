use bevy_ecs::prelude::*;
use engine::core::content::Content;
use engine::core::math::Offset;
use engine::core::time::{Seconds, WallClock};
use engine::systems::area;
use engine::systems::dialogue::{self, ConversationRequest};
use engine::systems::interact::{self, InteractRequest};
use engine::systems::item::{ItemDef, ItemStack};
use engine::systems::movement::{self, position};
use engine::systems::npc::{self, NpcDef};
use engine::systems::script::{self, Speaker};
use engine::systems::shop::{self, SaleQuote, SaleVerdict, ShopDef, ShopRequest, ShopView};
use serde_json::{Value, json};
use testkit::{Game, Map, Sim, path, said};

#[test]
fn asking_a_keeper_opens_the_shop_and_buying_trades_the_price_for_the_goods() {
    let mut sim = market();
    let player = sim.join(1);
    sim.give(player, "gold", 10);

    let opened = browse(&mut sim, 1, "merchant");
    assert_eq!(opened.shop, sim.id::<ShopDef>("wares"));

    let conn = sim.conn(1);
    let offer = offer_of(&sim, "wares", "potion");
    sim.send(conn, ShopRequest::Buy { offer });
    sim.tick();

    assert_eq!(sim.count(player, "gold"), 4);
    assert_eq!(sim.count(player, "potion"), 1);
}

#[test]
fn a_purchase_you_cannot_cover_changes_nothing_and_the_keeper_says_so() {
    let mut sim = market();
    let player = sim.join(1);
    sim.give(player, "gold", 12);
    let merchant = sim.npc("merchant");
    let opened = browse(&mut sim, 1, "merchant");
    let tonic = offer_of(&sim, "wares", "tonic");
    assert!(opened.offers[tonic as usize].refusal.is_some());

    let conn = sim.conn(1);
    sim.send(conn, ShopRequest::Buy { offer: tonic });
    sim.tick();

    assert_eq!(sim.count(player, "gold"), 12);
    assert_eq!(sim.count(player, "tonic"), 0);
    let talking = sim.conversation(1).expect("still browsing");
    assert_eq!(talking.with, Some(merchant));
    let remark = talking.remark.expect("the merchant reacts");
    assert_eq!(remark.line.by, Speaker::Npc(sim.id::<NpcDef>("merchant")));
}

#[test]
fn limited_stock_is_kept_per_player_and_restocks_on_its_timer() {
    let mut sim = market();
    let first = sim.join(1);
    let second = sim.join(2);
    for player in [first, second] {
        sim.give(player, "gold", 100);
        sim.give(player, "wing", 20);
    }
    browse(&mut sim, 1, "merchant");
    browse(&mut sim, 2, "merchant");
    let tonic = offer_of(&sim, "wares", "tonic");
    let (one, two) = (sim.conn(1), sim.conn(2));

    for _ in 0..4 {
        sim.send(one, ShopRequest::Buy { offer: tonic });
        sim.tick();
    }
    assert_eq!(sim.count(first, "tonic"), 3);
    let sold_out = shown(&sim, first).expect("open");
    assert_eq!(sold_out.offers[tonic as usize].left, Some(0));

    sim.send(two, ShopRequest::Buy { offer: tonic });
    sim.tick();
    assert_eq!(sim.count(second, "tonic"), 1);

    let mut clock = sim.world.resource_mut::<WallClock>();
    clock.now = clock.now.after(Seconds(600.0));
    sim.run(0.5);
    sim.send(one, ShopRequest::Buy { offer: tonic });
    sim.tick();
    assert_eq!(sim.count(first, "tonic"), 4);
}

#[test]
fn a_collector_pays_in_his_own_currency_and_buyback_costs_exactly_what_he_paid() {
    let mut sim = market();
    let player = sim.join(1);
    sim.give(player, "bone", 7);
    sim.give(player, "tusk", 1);

    let opened = browse(&mut sim, 1, "collector");
    assert_eq!(opened.shop, sim.id::<ShopDef>("exchange"));
    assert!(
        sim.conversation(1).is_some(),
        "a shop without reactions still keeps its conversation"
    );

    let conn = sim.conn(1);
    let tusk = sim.slot_of(player, "tusk");
    let one_tusk = ItemStack::new(sim.id::<ItemDef>("tusk"), 1);
    sim.send(
        conn,
        ShopRequest::Sell {
            slot: tusk,
            stack: one_tusk,
        },
    );
    sim.tick();
    assert_eq!(sim.count(player, "tusk"), 1);

    let bones = sim.slot_of(player, "bone");
    let all_bones = ItemStack::new(sim.id::<ItemDef>("bone"), 7);
    for _ in 0..2 {
        sim.send(
            conn,
            ShopRequest::Sell {
                slot: bones,
                stack: all_bones,
            },
        );
    }
    sim.tick();
    assert_eq!(sim.count(player, "bone"), 0);
    assert_eq!(sim.count(player, "token"), 7);
    assert_eq!(shown(&sim, player).expect("open").buyback.len(), 1);

    sim.send(conn, ShopRequest::Buyback { sale: 0 });
    sim.tick();
    assert_eq!(sim.count(player, "bone"), 7);
    assert_eq!(sim.count(player, "token"), 0);
    assert!(shown(&sim, player).expect("open").buyback.is_empty());
}

#[test]
fn the_view_quotes_what_each_bag_item_fetches_or_why_it_does_not_sell() {
    let mut sim = market();
    let player = sim.join(1);
    sim.give(player, "bone", 7);
    sim.give(player, "tusk", 1);

    browse(&mut sim, 1, "collector");

    let view = shown(&sim, player).expect("open");
    let token = sim.id::<ItemDef>("token");
    let bone = quote(&view, &sim, "bone").expect("quoted");
    assert!(matches!(
        &bone.verdict,
        SaleVerdict::Pays(pays) if pays.iter().any(|stack| stack.item == token)
    ));
    let tusk = quote(&view, &sim, "tusk").expect("quoted");
    assert!(matches!(tusk.verdict, SaleVerdict::Refused(_)));
}

#[test]
fn leaving_the_conversation_or_walking_out_of_reach_closes_the_shop() {
    let mut sim = market();
    let player = sim.join(1);
    browse(&mut sim, 1, "merchant");

    sim.leave(1);

    assert!(sim.conversation(1).is_none());
    assert!(shown(&sim, player).is_none());

    browse(&mut sim, 1, "collector");
    let at = position(&sim.world, player).expect("position");
    movement::relocate(&mut sim.world, player, at + Offset::new(0.0, 8.0));
    sim.tick();

    assert!(shown(&sim, player).is_none());
}

#[test]
fn a_map_object_can_keep_a_shop() {
    let mut sim = market();
    let player = sim.join(1);
    sim.give(player, "gold", 3);
    let honesty_box = sim.object("box");

    let conn = sim.conn(1);
    sim.send(
        conn,
        InteractRequest {
            target: honesty_box,
        },
    );
    assert!(sim.run_until(20.0, |world| world.get::<ShopView>(player).is_some()));
    assert_eq!(
        shown(&sim, player).map(|opened| opened.shop),
        Some(sim.id::<ShopDef>("box"))
    );
    assert!(
        sim.conversation(1).is_some(),
        "the box keeps its shop inside a conversation"
    );

    let offer = offer_of(&sim, "box", "steak");
    sim.send(conn, ShopRequest::Buy { offer });
    sim.tick();
    assert_eq!(sim.count(player, "steak"), 1);
    assert_eq!(sim.count(player, "gold"), 0);
}

fn market() -> Sim {
    let content = Game::new()
        .area(
            "market",
            &Map::open(15, 15),
            json!([
                { "type": "resident", "npc": "merchant.npc.json", "at": { "x": 4.0, "y": 4.0 } },
                { "type": "resident", "npc": "collector.npc.json", "at": { "x": 10.0, "y": 4.0 } },
                {
                    "type": "object",
                    "name": "box",
                    "title": "Honesty box",
                    "shape": { "type": "point", "x": 7.0, "y": 10.0 },
                    "look": path("prop", "box"),
                    "scripts": [{ "script": "box_use.script.json" }],
                },
            ]),
        )
        .script(
            "box_use",
            "object",
            json!({
                "interacted": { "type": "on", "event": "interacted", "verb": "use", "reach": 1.5, "next": "note" },
                "note": said("Pay what's written.", "shop"),
                "shop": { "type": "open_shop", "shop": "box.shop.json" },
            }),
        )
        .prop("box", json!({ "type": "none" }))
        .hero("market", 7.0, 7.0)
        .item("gold", json!({ "name": "Gold", "kind": { "type": "resource" }, "modules": [{ "type": "stack", "max": 100_000 }] }))
        .resource("potion", 1000)
        .resource("tonic", 1000)
        .resource("wing", 1000)
        .resource("bone", 1000)
        .resource("token", 1000)
        .resource("tusk", 1000)
        .resource("steak", 1000)
        .npc("merchant", 40.0, json!([{ "type": "script", "script": "merchant_talk.script.json" }]))
        .script(
            "merchant_talk",
            "npc",
            keeper("Show me your wares.", "wares", "wares"),
        )
        .npc("collector", 40.0, json!([{ "type": "script", "script": "collector_talk.script.json" }]))
        .script(
            "collector_talk",
            "npc",
            keeper("I've brought bones.", "counter", "exchange"),
        )
        .row(
            "shop",
            "wares",
            json!({
                "title": "Wares",
                "mark": "merchant.attention.json",
                "modules": [
                    {
                        "type": "sells",
                        "item": "potion.item.json",
                        "price": [{ "item": "gold.item.json", "count": 6 }],
                        "stock": { "type": "unlimited" },
                    },
                    {
                        "type": "sells",
                        "item": "tonic.item.json",
                        "price": [{ "item": "gold.item.json", "count": 10 }, { "item": "wing.item.json", "count": 2 }],
                        "stock": { "type": "limited", "count": 3, "restock": 600.0 },
                    },
                    {
                        "type": "reactions",
                        "bought": [{ "by": { "type": "npc", "npc": "merchant.npc.json" }, "text": ["A fine choice."] }],
                        "sold": [{ "by": { "type": "npc", "npc": "merchant.npc.json" }, "text": ["It'll do."] }],
                        "cant_afford": [{
                            "by": { "type": "npc", "npc": "merchant.npc.json" },
                            "text": ["Come back when your purse is heavier."],
                        }],
                    },
                ],
            }),
        )
        .row(
            "shop",
            "exchange",
            json!({
                "title": "Exchange",
                "mark": "collector.attention.json",
                "modules": [
                    {
                        "type": "sells",
                        "item": "tonic.item.json",
                        "price": [{ "item": "token.item.json", "count": 3 }],
                        "stock": { "type": "unlimited" },
                    },
                    { "type": "buys", "what": { "type": "item", "item": "bone.item.json" }, "pays": [{ "item": "token.item.json" }] },
                ],
            }),
        )
        .row(
            "shop",
            "box",
            json!({
                "title": "Honesty box",
                "mark": "merchant.attention.json",
                "modules": [{
                    "type": "sells",
                    "item": "steak.item.json",
                    "price": [{ "item": "gold.item.json", "count": 3 }],
                    "stock": { "type": "limited", "count": 5, "restock": 1800.0 },
                }],
            }),
        )
        .mark("merchant")
        .mark("collector")
        .sfx("ui_refuse")
        .sfx("ui_close")
        .sfx("coins")
        .load();
    let mut sim = Sim::new(content);
    sim.serve("market")
        .accept::<InteractRequest>()
        .accept::<ConversationRequest>()
        .accept::<ShopRequest>()
        .add((
            interact::interact_request,
            dialogue::requests,
            shop::requests,
        ))
        .add((
            script::run_tick,
            movement::advance,
            interact::interactions,
            dialogue::hold,
            dialogue::refresh,
            shop::hold,
            shop::refresh,
        ));
    script::spawn_stateless(&mut sim.world);
    npc::spawn_all(&mut sim.world);
    area::object::spawn_all(&mut sim.world);
    sim
}

fn keeper(ask: &str, browsing: &str, shop: &str) -> Value {
    let mut nodes = json!({
        "interacted": { "type": "on", "event": "interacted", "verb": "talk", "reach": 2.0, "next": "hello" },
        "hello": {
            "type": "dialogue",
            "lines": [{ "by": { "type": "narrator" }, "text": ["Welcome."] }],
            "choices": [{ "label": [ask], "to": browsing }, { "label": ["Goodbye."] }],
        },
        "shop": { "type": "open_shop", "shop": path("shop", shop) },
    });
    nodes[browsing] = said("Take your time.", "shop");
    nodes
}

fn browse(sim: &mut Sim, client: u32, keeper: &str) -> ShopView {
    let player = sim.player(client);
    sim.talk(client, keeper);
    let ask = match keeper {
        "merchant" => "Show me your wares.",
        "collector" => "I've brought bones.",
        other => panic!("{other} keeps no shop"),
    };
    sim.pick(client, ask);
    assert!(
        sim.run_until(20.0, |world| world.get::<ShopView>(player).is_some()),
        "the shop with the {keeper} never opened"
    );
    shown(sim, player).expect("the shop opened")
}

fn shown(sim: &Sim, player: Entity) -> Option<ShopView> {
    sim.world.get::<ShopView>(player).cloned()
}

fn quote<'a>(view: &'a ShopView, sim: &Sim, item: &str) -> Option<&'a SaleQuote> {
    let item = sim.id::<ItemDef>(item);
    view.quotes.iter().find(|quote| quote.item == item)
}

fn offer_of(sim: &Sim, shop: &str, item: &str) -> u32 {
    let content = sim.world.resource::<Content>();
    let item = sim.id::<ItemDef>(item);
    sim.id::<ShopDef>(shop)
        .get(content)
        .sells()
        .position(|offer| offer.item == item)
        .expect("on sale") as u32
}
