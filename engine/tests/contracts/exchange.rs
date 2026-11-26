use engine::core::content::Content;
use engine::systems::item::bag_view::{BagFilter, categories_used};
use engine::systems::item::effects::move_between;
use engine::systems::item::{
    self, BagSort, ExchangeRefusal, Inventory, ItemCategory, ItemDef, ItemId, ItemStack,
    MoveStackRequest, SortBagRequest, SplitStackRequest,
};
use engine::systems::messages;
use serde_json::json;
use testkit::{Declared, GEAR, Game, Map, Sim, path};

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
fn a_player_arranges_their_bag_and_its_stacks_keep_their_slots() {
    let mut sim = Sim::new(
        goods()
            .sfx("ui_refuse")
            .area("market", &Map::open(5, 5), json!([]))
            .hero("market", 2.0, 2.0)
            .load(),
    );
    sim.serve("market")
        .accept::<SplitStackRequest>()
        .accept::<MoveStackRequest>()
        .accept::<SortBagRequest>()
        .add(item::arrange_stacks);
    let player = sim.join(1);
    let conn = sim.conn(1);
    let [tusk, bone] = ["tusk", "bone"].map(|key| sim.id::<ItemDef>(key));
    let held = |sim: &Sim| sim.world.get::<Inventory>(player).expect("a bag").clone();
    let laid = |sim: &Sim| -> Vec<(usize, ItemStack)> { held(sim).stacks().collect() };
    let at = |slot, item, count| (slot, ItemStack::new(item, count));
    let moved = |sim: &mut Sim, from, to, item| {
        sim.send(conn, MoveStackRequest { from, to, item });
        sim.tick();
    };
    sim.give(player, "tusk", 70);
    sim.give(player, "bone", 1);
    assert_eq!(
        laid(&sim),
        [at(0, tusk, 50), at(1, tusk, 20), at(2, bone, 1)],
        "new items fill their stacks, then the first empty slots"
    );

    sim.send(conn, SplitStackRequest { slot: 0, count: 10 });
    sim.tick();
    let split = [
        at(0, tusk, 40),
        at(1, tusk, 20),
        at(2, bone, 1),
        at(3, tusk, 10),
    ];
    assert_eq!(
        laid(&sim),
        split,
        "a split takes the first empty slot after its stack"
    );
    for count in [0, 40, 41] {
        sim.send(conn, SplitStackRequest { slot: 0, count });
    }
    sim.tick();
    assert_eq!(laid(&sim), split, "a split leaves both stacks something");

    moved(&mut sim, 3, 6, tusk);
    moved(&mut sim, 2, 0, bone);
    assert_eq!(
        laid(&sim),
        [
            at(0, bone, 1),
            at(1, tusk, 20),
            at(2, tusk, 40),
            at(6, tusk, 10)
        ],
        "a stack dropped on an empty slot moves there, and one dropped on another item swaps"
    );

    moved(&mut sim, 6, 1, tusk);
    moved(&mut sim, 2, 1, tusk);
    assert_eq!(
        laid(&sim),
        [at(0, bone, 1), at(1, tusk, 50), at(2, tusk, 20)],
        "stacks of one item join, and what the stack has no room for stays where it was"
    );
    moved(&mut sim, 2, 1, tusk);
    assert_eq!(
        laid(&sim),
        [at(0, bone, 1), at(1, tusk, 20), at(2, tusk, 50)],
        "a stack dropped on a full stack of its item swaps with it"
    );

    let before = laid(&sim);
    moved(&mut sim, 0, 5, tusk);
    assert_eq!(
        laid(&sim),
        before,
        "a move of a stack the bag no longer holds is refused"
    );
    assert_eq!(
        sim.errors(1),
        ["That stack changed before it could be moved."]
    );

    sim.take(player, "bone", 1);
    sim.give(player, "tusk", 35);
    assert_eq!(
        laid(&sim),
        [at(0, tusk, 5), at(1, tusk, 50), at(2, tusk, 50)],
        "a stack that runs out leaves its slot empty for the next new stack"
    );

    let free = held(&sim).free_slots();
    sim.give(player, "bone", free - 1);
    let last = held(&sim).size() as usize - 1;
    moved(&mut sim, 2, last as u32, tusk);
    sim.send(
        conn,
        SplitStackRequest {
            slot: last as u32,
            count: 5,
        },
    );
    sim.tick();
    assert_eq!(
        held(&sim).at(2),
        Some(ItemStack::new(tusk, 5)),
        "a split past the last slot wraps around to the first empty one"
    );
    let full = held(&sim);
    sim.send(conn, SplitStackRequest { slot: 1, count: 5 });
    sim.tick();
    assert_eq!(held(&sim), full, "a full bag has no slot for a new stack");
}

#[test]
fn a_sort_lays_the_whole_bag_out_once_in_the_order_picked() {
    let mut sim = Sim::new(
        goods()
            .rarity("rare", 3, json!({}))
            .item(
                "amber",
                json!({ "name": "Amber", "rarity": path("item_rarity", "rare"), "kind": { "type": "resource" }, "modules": [{ "type": "stack", "max": 50 }] }),
            )
            .equipment(
                "crown",
                json!({ "name": "Crown", "rarity": path("item_rarity", "rare"), "kind": { "type": "equipment", "category": path("equipment_category", GEAR), "wear": "held", "stance": "sword.stance.json" } }),
            )
            .item(
                "draught",
                json!({
                    "name": "draught",
                    "kind": { "type": "usable" },
                    "modules": [{ "type": "script", "script": "draught_drunk.script.json" }],
                }),
            )
            .script(
                "draught_drunk",
                "item",
                json!({ "used": { "type": "on", "event": "used", "next": "heal" }, "heal": { "type": "heal", "by": "full" } }),
            )
            .area("market", &Map::open(5, 5), json!([]))
            .hero("market", 2.0, 2.0)
            .load(),
    );
    sim.serve("market")
        .accept::<SplitStackRequest>()
        .accept::<MoveStackRequest>()
        .accept::<SortBagRequest>()
        .add(item::arrange_stacks);
    let player = sim.join(1);
    let conn = sim.conn(1);
    let [tusk, bone, amber, crown, draught, sword] =
        ["tusk", "bone", "amber", "crown", "draught", "sword"].map(|key| sim.id::<ItemDef>(key));
    for (key, count) in [
        ("bone", 1),
        ("tusk", 30),
        ("sword", 1),
        ("amber", 4),
        ("draught", 1),
    ] {
        sim.give(player, key, count);
    }
    sim.send(
        conn,
        MoveStackRequest {
            from: 1,
            to: 9,
            item: tusk,
        },
    );
    sim.tick();
    sim.give(player, "tusk", 30);
    sim.give(player, "crown", 1);
    let sorted = |sim: &mut Sim, by| {
        sim.send(conn, SortBagRequest { by });
        sim.tick();
        sim.world
            .get::<Inventory>(player)
            .expect("a bag")
            .slots
            .clone()
    };
    let laid = |stacks: &[(ItemId, u32)], size: usize| -> Vec<Option<ItemStack>> {
        let mut slots: Vec<Option<ItemStack>> = stacks
            .iter()
            .map(|&(item, count)| Some(ItemStack::new(item, count)))
            .collect();
        slots.resize(size, None);
        slots
    };
    let size = sim.world.get::<Inventory>(player).expect("a bag").size() as usize;

    assert_eq!(
        sorted(&mut sim, BagSort::Kind),
        laid(
            &[
                (draught, 1),
                (crown, 1),
                (sword, 1),
                (amber, 4),
                (bone, 1),
                (tusk, 50),
                (tusk, 10)
            ],
            size
        ),
        "by kind: consumables, equipment, then materials, the rarest first in each, then by name, \
         each item's stacks joined and laid from the first slot with no gaps"
    );
    assert_eq!(
        sorted(&mut sim, BagSort::Rarity),
        laid(
            &[
                (crown, 1),
                (amber, 4),
                (draught, 1),
                (sword, 1),
                (bone, 1),
                (tusk, 50),
                (tusk, 10)
            ],
            size
        ),
        "by rarity: the rarest first, then by kind and name"
    );
    let by_name = laid(
        &[
            (amber, 4),
            (bone, 1),
            (crown, 1),
            (draught, 1),
            (sword, 1),
            (tusk, 50),
            (tusk, 10),
        ],
        size,
    );
    assert_eq!(
        sorted(&mut sim, BagSort::Name),
        by_name,
        "by name, whatever the case of its first letter, the larger stack of an item first"
    );
    sim.send(
        conn,
        MoveStackRequest {
            from: 0,
            to: 12,
            item: amber,
        },
    );
    sim.tick();
    assert_eq!(
        sorted(&mut sim, BagSort::Name),
        by_name,
        "the same bag always sorts the same way"
    );
}

#[test]
fn the_bag_search_shows_the_stacks_every_word_names_of_the_type_picked() {
    let content = goods()
        .item(
            "tusk_old",
            json!({ "name": "Old Tusk", "kind": { "type": "resource" } }),
        )
        .load();
    let shown = |words: &str, category| {
        let filter = BagFilter {
            words: words.to_owned(),
            category,
        };
        let mut names: Vec<String> = content
            .ids::<ItemDef>()
            .map(|item| item.get(&content))
            .filter(|def| filter.shows(def))
            .map(|def| def.name.clone())
            .collect();
        names.sort();
        names
    };

    assert_eq!(
        shown("", None).len(),
        5,
        "nothing typed and every type shows the whole bag"
    );
    assert_eq!(
        shown("TUSK", None),
        ["Old Tusk", "tusk"],
        "the words match whatever their case"
    );
    assert_eq!(
        shown("tusk old", None),
        ["Old Tusk"],
        "every word typed must be in the name"
    );
    assert_eq!(
        shown("", Some(ItemCategory::Equipment)),
        ["sword"],
        "the type picked narrows it"
    );
    assert!(
        shown("gold", Some(ItemCategory::Equipment)).is_empty(),
        "both must hold"
    );
    assert_eq!(
        categories_used(&content),
        [ItemCategory::Equipment, ItemCategory::Material],
        "the type picker offers the types the game's items are of"
    );
    let played = Game::new()
        .area("start", &Map::open(4, 4), json!([]))
        .hero("start", 1.0, 1.0)
        .load();
    for (category, name) in [
        (ItemCategory::Consumable, "Consumable"),
        (ItemCategory::Equipment, "Equipment"),
        (ItemCategory::Material, "Material"),
        (ItemCategory::Currency, "Currency"),
    ] {
        assert_eq!(
            category.named(&messages::of(&played).item),
            name,
            "the type picker names each type as the game's messages do"
        );
    }
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
        .equipment(
            "sword",
            json!({ "kind": { "type": "equipment", "category": path("equipment_category", GEAR), "wear": "held", "stance": "sword.stance.json" } }),
        )
}

fn items<const N: usize>(content: &Content, keys: [&str; N]) -> [ItemId; N] {
    keys.map(|key| content.declared::<ItemDef>(key).expect("a declared item"))
}

fn bag(size: u32, stacks: &[ItemStack]) -> Inventory {
    let mut bag = Inventory::sized(size);
    for (slot, &stack) in bag.slots.iter_mut().zip(stacks) {
        *slot = Some(stack);
    }
    bag
}
