use engine::core::content::Content;
use engine::systems::item::effects::move_between;
use engine::systems::item::{
    self, ExchangeRefusal, Inventory, ItemDef, ItemId, ItemStack, JoinStacksRequest,
    SplitStackRequest,
};
use serde_json::json;
use testkit::{Declared, Game, Map, Sim};

#[test]
fn moving_items_onto_the_same_character_changes_nothing() {
    let mut sim = Sim::new(
        goods()
            .area("market", &Map::open(5, 5), json!([]))
            .hero("market", 2.0, 2.0)
            .load(),
    );
    sim.serve("market");
    let player = sim.join(1);
    sim.give(player, "gold", 10);
    let gold = sim.id::<ItemDef>("gold");

    let result = move_between(&mut sim.world, player, player, &[ItemStack::new(gold, 4)]);

    assert!(result.is_err());
    assert_eq!(sim.count(player, "gold"), 10);
}

#[test]
fn a_hand_in_frees_the_slot_its_reward_needs() {
    let content = goods().load();
    let [tusk, bone, sword] = items(&content, ["tusk", "bone", "sword"]);
    let mut inventory = bag(2, &[ItemStack::new(tusk, 5), ItemStack::new(bone, 1)]);

    let result = inventory.exchange(
        &content,
        &[ItemStack::new(tusk, 5)],
        &[ItemStack::new(sword, 1)],
    );

    assert_eq!(result, Ok(()));
    assert_eq!(inventory.count(tusk), 0);
    assert_eq!(inventory.count(sword), 1);
    assert_eq!(inventory.count(bone), 1);
}

#[test]
fn a_refused_exchange_changes_nothing() {
    let content = goods().load();
    let [gold, bone, sword] = items(&content, ["gold", "bone", "sword"]);
    let original = bag(2, &[ItemStack::new(gold, 10), ItemStack::new(bone, 1)]);

    let mut short = original.clone();
    assert_eq!(
        short.exchange(&content, &[ItemStack::new(gold, 15)], &[]),
        Err(ExchangeRefusal::Missing(ItemStack::new(gold, 5)))
    );
    assert_eq!(short, original);

    let mut full = original.clone();
    assert_eq!(
        full.exchange(
            &content,
            &[ItemStack::new(gold, 1)],
            &[ItemStack::new(sword, 1)]
        ),
        Err(ExchangeRefusal::NoRoom { slots: 1 })
    );
    assert_eq!(full, original);
}

#[test]
fn costs_of_the_same_item_add_up() {
    let content = goods().load();
    let [gold] = items(&content, ["gold"]);
    let mut inventory = bag(5, &[ItemStack::new(gold, 20)]);
    assert_eq!(
        inventory.exchange(
            &content,
            &[ItemStack::new(gold, 15), ItemStack::new(gold, 10)],
            &[]
        ),
        Err(ExchangeRefusal::Missing(ItemStack::new(gold, 5)))
    );
}

#[test]
fn a_stack_without_limit_holds_every_coin_in_one_slot() {
    let content = goods().load();
    let [gold] = items(&content, ["gold"]);
    let mut inventory = bag(1, &[]);
    inventory
        .exchange(&content, &[], &[ItemStack::new(gold, u32::MAX - 10)])
        .expect("gold fits");
    inventory
        .exchange(&content, &[], &[ItemStack::new(gold, 10)])
        .expect("gold still fits in the one slot");
    assert_eq!(inventory.count(gold), u32::MAX);
}

#[test]
fn refusals_read_as_what_is_needed() {
    let content = goods()
        .area("field", &Map::open(4, 4), json!([]))
        .hero("field", 1.0, 1.0)
        .load();
    let [gold] = items(&content, ["gold"]);
    assert_eq!(
        ExchangeRefusal::NoRoom { slots: 1 }
            .line(&content)
            .plain_words(&content),
        "Needs 1 free slot"
    );
    assert_eq!(
        ExchangeRefusal::Missing(ItemStack::new(gold, 20))
            .line(&content)
            .plain_words(&content),
        "Needs 20 Gold more"
    );
}

#[test]
fn taking_up_to_stacks_takes_what_is_held_and_no_more() {
    let content = goods().load();
    let [tusk, bone, gold] = items(&content, ["tusk", "bone", "gold"]);
    let mut inventory = bag(3, &[ItemStack::new(tusk, 3), ItemStack::new(bone, 1)]);

    inventory.take_up_to(&[
        ItemStack::new(tusk, 2),
        ItemStack::new(tusk, 2),
        ItemStack::new(gold, 5),
    ]);

    assert_eq!(inventory.count(tusk), 0);
    assert_eq!(inventory.count(gold), 0);
    assert_eq!(inventory.count(bone), 1);
}

#[test]
fn a_player_splits_a_stack_in_two_and_joins_stacks_as_far_as_one_holds() {
    let mut sim = Sim::new(
        goods()
            .sfx("ui_refuse")
            .area("market", &Map::open(5, 5), json!([]))
            .hero("market", 2.0, 2.0)
            .load(),
    );
    sim.serve("market")
        .accept::<SplitStackRequest>()
        .accept::<JoinStacksRequest>()
        .add(item::arrange_stacks);
    let player = sim.join(1);
    let conn = sim.conn(1);
    let [tusk, bone] = ["tusk", "bone"].map(|key| sim.id::<ItemDef>(key));
    let held = |sim: &Sim| sim.world.get::<Inventory>(player).expect("a bag").clone();
    sim.give(player, "tusk", 70);
    sim.give(player, "bone", 1);

    sim.send(conn, SplitStackRequest { slot: 0, count: 10 });
    sim.tick();
    let split = [
        ItemStack::new(tusk, 40),
        ItemStack::new(tusk, 10),
        ItemStack::new(tusk, 20),
        ItemStack::new(bone, 1),
    ];
    assert_eq!(held(&sim).slots, split, "the new stack lies beside its own");

    for count in [0, 40, 41] {
        sim.send(conn, SplitStackRequest { slot: 0, count });
    }
    sim.send(conn, JoinStacksRequest { from: 3, onto: 0 });
    sim.tick();
    assert_eq!(
        held(&sim).slots,
        split,
        "a split leaves both stacks something, and only stacks of one item join"
    );

    sim.send(conn, JoinStacksRequest { from: 2, onto: 1 });
    sim.tick();
    sim.send(conn, JoinStacksRequest { from: 1, onto: 0 });
    sim.tick();
    let joined = [
        ItemStack::new(tusk, 50),
        ItemStack::new(tusk, 20),
        ItemStack::new(bone, 1),
    ];
    assert_eq!(
        held(&sim).slots,
        joined,
        "what a full stack has no room for stays where it was"
    );

    let free = held(&sim).free_slots();
    sim.give(player, "bone", free);
    let full = held(&sim);
    sim.send(conn, SplitStackRequest { slot: 1, count: 5 });
    sim.tick();
    assert_eq!(held(&sim), full, "a full bag has no slot for a new stack");
}

fn goods() -> Game {
    Game::new()
        .item(
            "gold",
            json!({
                "name": "Gold",
                "kind": { "type": "resource" },
                "modules": [{ "type": "stack", "max": u32::MAX }],
            }),
        )
        .item(
            "tusk",
            json!({ "kind": { "type": "resource" }, "modules": [{ "type": "stack", "max": 50 }] }),
        )
        .item("bone", json!({ "kind": { "type": "resource" } }))
        .stance("sword", 1, "main")
        .item(
            "sword",
            json!({ "kind": { "type": "equipment", "wear": "held", "stance": "sword.stance.json" } }),
        )
}

fn items<const N: usize>(content: &Content, keys: [&str; N]) -> [ItemId; N] {
    keys.map(|key| content.declared::<ItemDef>(key).expect("a declared item"))
}

fn bag(max: u32, slots: &[ItemStack]) -> Inventory {
    Inventory {
        slots: slots.to_vec(),
        max,
    }
}
