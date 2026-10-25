use bevy_ecs::prelude::*;
use engine::systems::dialogue::{self, ConversationPanel, ConversationRequest};
use engine::systems::interact::{self, InteractRequest};
use engine::systems::item::{
    self, DropItemRequest, DroppedItem, INVENTORY_MAX, Inventory, ItemDef, ItemStack,
    UseItemRequest,
};
use engine::systems::memory::{self, MemoryDef};
use engine::systems::player::Xp;
use engine::systems::quest::{self, QuestDef, QuestLog, QuestRequest, QuestResult};
use engine::systems::shop::{self, ShopRequest};
use engine::systems::{area, combat, movement, npc, rewards, script, stat};
use serde_json::{Value, json};
use testkit::{Game, Map, Sim, dialogue, labels, locked, path, said};

#[test]
fn accepting_a_quest_logs_it_tracks_it_and_hands_over_its_items() {
    let mut sim = harbour();
    let player = sim.join(1);

    let greeting = sim.talk(1, "giver");
    assert!(labels(&greeting).contains(&"A Letter".to_owned()));
    let offer = sim.pick(1, "A Letter").expect("the offer");
    assert!(labels(&offer).contains(&"I'll take it.".to_owned()));
    sim.pick(1, "I'll take it.");

    let log = log(&sim, player);
    let letter = sim.id::<QuestDef>("letter");
    assert!(log.active(letter).is_some());
    assert!(log.tracked.contains(&letter));
    assert_eq!(sim.count(player, "letter"), 1);
}

#[test]
fn abandoning_takes_the_quest_items_back_and_accepting_again_returns_them() {
    let mut sim = harbour();
    let player = sim.join(1);
    accept(&mut sim, player, "letter");
    assert_eq!(sim.count(player, "letter"), 1);

    let conn = sim.conn(1);
    let quest = sim.id::<QuestDef>("letter");
    sim.send(conn, QuestRequest::Abandon { quest });
    sim.tick();
    assert!(log(&sim, player).active.is_empty());
    assert_eq!(sim.count(player, "letter"), 0);

    sim.talk(1, "giver");
    sim.pick(1, "A Letter");
    sim.pick(1, "I'll take it.");
    assert_eq!(sim.count(player, "letter"), 1);
}

#[test]
fn delivering_completes_the_quest_once_and_with_the_level_unlocks_the_next_in_the_chain() {
    let mut sim = harbour();
    let player = sim.join(1);
    let levelled = sim.join(2);
    level_up(&mut sim, levelled);
    let captain = sim.talk(2, "captain");
    assert!(!labels(&captain).contains(&"Bats".to_owned()));

    sim.talk(1, "giver");
    sim.pick(1, "A Letter");
    sim.pick(1, "I'll take it.");
    sim.leave(1);
    sim.run(0.5);
    let gold = sim.count(player, "gold");

    let captain = sim.talk(1, "captain");
    assert!(!labels(&captain).contains(&"Bats".to_owned()));
    let thanks = sim.pick(1, "A Letter").expect("thanks");
    let letter = sim.id::<QuestDef>("letter");
    assert_eq!(thanks.panel, Some(ConversationPanel::QuestTurnIn(letter)));
    let stale = thanks.step;
    sim.pick(1, "The giver asked me to bring you this.");

    assert_eq!(sim.count(player, "letter"), 0);
    assert_eq!(sim.count(player, "gold"), gold + 10);
    assert_eq!(
        log(&sim, player).finished(letter).map(|done| done.result),
        Some(QuestResult::Completed)
    );

    let conn = sim.conn(1);
    sim.send(
        conn,
        ConversationRequest::Pick {
            step: stale,
            choice: 0,
        },
    );
    sim.tick();
    assert_eq!(sim.count(player, "gold"), gold + 10);

    sim.leave(1);
    let again = sim.talk(1, "captain");
    assert!(!labels(&again).contains(&"A Letter".to_owned()));
    assert!(!labels(&again).contains(&"Bats".to_owned()));

    sim.leave(1);
    level_up(&mut sim, player);
    let ready = sim.talk(1, "captain");
    assert!(!locked(&ready, "Bats"));
}

#[test]
fn kills_count_only_for_the_credited_player() {
    let mut sim = harbour();
    let hunter = sim.join(1);
    let bystander = sim.join(2);
    for player in [hunter, bystander] {
        qualify_for_bats(&mut sim, player);
        accept(&mut sim, player, "bats");
    }

    slay(&mut sim, hunter, "bat");
    slay(&mut sim, hunter, "rat");

    let bats = sim.id::<QuestDef>("bats");
    let defeated = |sim: &Sim, player| log(sim, player).active(bats).map(|on| on.counts[0]);
    assert_eq!(defeated(&sim, hunter), Some(1));
    assert_eq!(defeated(&sim, bystander), Some(0));
}

#[test]
fn quest_drops_fall_only_for_players_on_the_quest() {
    let mut sim = harbour();
    let player = sim.join(1);
    let key = sim.id::<ItemDef>("key");
    let keys = |sim: &mut Sim| {
        sim.world
            .query::<&DroppedItem>()
            .iter(&sim.world)
            .filter(|dropped| dropped.item == key)
            .count()
    };

    for _ in 0..8 {
        slay(&mut sim, player, "bat");
    }
    assert_eq!(keys(&mut sim), 0);

    qualify_for_bats(&mut sim, player);
    accept(&mut sim, player, "bats");
    for _ in 0..8 {
        slay(&mut sim, player, "bat");
    }
    assert!(keys(&mut sim) >= 1);
}

#[test]
fn quest_items_never_sell_or_drop() {
    let mut sim = harbour();
    let player = sim.join(1);
    accept(&mut sim, player, "letter");
    sim.give(player, "tusk", 1);
    let conn = sim.conn(1);

    let slot = sim.slot_of(player, "letter");
    sim.send(conn, DropItemRequest { slot });
    sim.tick();
    assert_eq!(sim.count(player, "letter"), 1);

    sim.talk(1, "trader");
    sim.pick(1, "Show me your wares.");
    for kept in ["letter", "tusk"] {
        let slot = sim.slot_of(player, kept);
        let stack = ItemStack::new(sim.id::<ItemDef>(kept), 1);
        sim.send(conn, ShopRequest::Sell { slot, stack });
        sim.tick();
    }
    assert_eq!(sim.count(player, "letter"), 1);
    assert_eq!(sim.count(player, "tusk"), 0);
}

#[test]
fn a_full_bag_refuses_the_reward_counting_space_after_the_hand_in() {
    let mut sim = harbour();
    let player = sim.join(1);
    accept(&mut sim, player, "tusks");
    sim.give(player, "gold", 1);
    sim.give(player, "tusk", 7);
    fill_bag(&mut sim, player);
    sim.run(0.5);

    sim.talk(1, "trader");
    let thanks = sim.pick(1, "Tusks").expect("thanks");
    let tusks = sim.id::<QuestDef>("tusks");
    assert_eq!(thanks.panel, Some(ConversationPanel::QuestTurnIn(tusks)));
    assert!(!locked(&thanks, "I'll take the shield."));
    let refused = sim
        .pick(1, "I'll take the shield.")
        .and_then(|now| now.refused);
    assert!(refused.is_some());
    assert_eq!(
        sim.errors(1).last().map(String::as_str),
        Some("Needs 1 free slot")
    );
    assert_eq!(sim.count(player, "tusk"), 7);
    assert_eq!(sim.count(player, "shield"), 0);
    sim.leave(1);

    let conn = sim.conn(1);
    let slot = sim.slot_of(player, "tusk");
    sim.send(conn, DropItemRequest { slot });
    sim.tick();
    sim.give(player, "tusk", 5);
    sim.run(0.5);

    sim.talk(1, "trader");
    sim.pick(1, "Tusks");
    sim.pick(1, "I'll take the shield.");
    assert_eq!(sim.count(player, "tusk"), 0);
    assert_eq!(sim.count(player, "shield"), 1);
    assert_eq!(
        log(&sim, player).finished(tusks).map(|done| done.result),
        Some(QuestResult::Completed)
    );
}

#[test]
fn a_daily_quest_returns_after_the_reset() {
    let mut sim = harbour();
    let player = sim.join(1);
    accept(&mut sim, player, "tithe");
    sim.give(player, "bone", 10);
    sim.run(0.5);

    sim.talk(1, "keeper");
    sim.pick(1, "Tithe");
    sim.pick(1, "Here are today's bones.");
    assert_eq!(sim.count(player, "token"), 6);

    let today = sim.talk(1, "keeper");
    assert!(!labels(&today).contains(&"Tithe".to_owned()));
    sim.leave(1);

    sim.later(24.0 * 3600.0);
    let tomorrow = sim.talk(1, "keeper");
    assert!(labels(&tomorrow).contains(&"Tithe".to_owned()));
}

#[test]
fn a_timed_quest_fails_when_time_runs_out_and_is_offered_again() {
    let mut sim = harbour();
    let player = sim.join(1);
    sim.talk(1, "giver");
    sim.pick(1, "Low Tide");
    sim.pick(1, "I'll hurry.");
    let tide = sim.id::<QuestDef>("low_tide");
    assert!(log(&sim, player).active(tide).is_some());

    sim.later(601.0);

    let failed = log(&sim, player);
    assert!(failed.active(tide).is_none());
    assert_eq!(
        failed.finished(tide).map(|done| done.result),
        Some(QuestResult::Failed)
    );
    assert!(failed.tracked.contains(&tide));
    let again = sim.talk(1, "giver");
    assert!(labels(&again).contains(&"Low Tide".to_owned()));
}

#[test]
fn an_item_starts_its_quest_and_offers_it_again_after_abandoning() {
    let mut sim = harbour();
    let player = sim.join(1);
    sim.give(player, "map", 1);
    let conn = sim.conn(1);
    let quest = sim.id::<QuestDef>("x_marks");

    let slot = sim.slot_of(player, "map");
    sim.send(conn, UseItemRequest { slot });
    assert!(sim.run_until(5.0, |world| {
        world
            .get::<dialogue::Conversation>(player)
            .is_some_and(|now| now.panel.is_some())
    }));
    let offer = sim.conversation(1).expect("the map speaks");
    assert_eq!(offer.panel, Some(ConversationPanel::QuestOffer(quest)));
    assert_eq!(offer.with, None);
    sim.pick(1, "Accept");
    assert!(log(&sim, player).active(quest).is_some());
    assert_eq!(sim.count(player, "map"), 1);

    sim.send(conn, QuestRequest::Abandon { quest });
    sim.tick();
    sim.send(conn, UseItemRequest { slot });
    assert!(sim.run_until(5.0, |world| {
        world
            .get::<dialogue::Conversation>(player)
            .is_some_and(|now| now.panel == Some(ConversationPanel::QuestOffer(quest)))
    }));
}

#[test]
fn exploring_the_marked_spot_readies_the_quest_for_the_object_there() {
    let mut sim = harbour();
    let player = sim.join(1);
    sim.give(player, "map", 1);
    accept(&mut sim, player, "x_marks");
    sim.run(0.5);
    let quest = sim.id::<QuestDef>("x_marks");

    let stone = sim.object("stone");
    sim.open_with(1, stone);
    assert!(sim.run_until(5.0, |world| {
        world
            .get::<dialogue::Conversation>(player)
            .is_some_and(|now| now.panel.is_some())
    }));
    assert_eq!(
        sim.conversation(1).and_then(|now| now.panel),
        Some(ConversationPanel::QuestTurnIn(quest))
    );

    sim.pick(1, "Dig where the cross says.");
    assert_eq!(sim.count(player, "map"), 0);
    assert_eq!(sim.count(player, "cutlass"), 1);
}

#[test]
fn deciding_fails_the_other_quest_and_is_remembered() {
    let mut sim = harbour();
    let player = sim.join(1);
    accept(&mut sim, player, "tusks");
    sim.give(player, "tusk", 5);
    sim.run(0.5);

    sim.talk(1, "shaman");
    sim.pick(1, "The Plea");
    sim.pick(1, "I'm listening.");
    let given = sim.pick(1, "Give her the tusks.").expect("her thanks");
    assert_eq!(labels(&given), vec!["Rest easy.".to_owned()]);

    let tusks = sim.id::<QuestDef>("tusks");
    let plea = sim.id::<QuestDef>("plea");
    let answered = log(&sim, player);
    assert_eq!(
        answered.finished(tusks).map(|done| done.result),
        Some(QuestResult::Failed)
    );
    assert_eq!(
        answered.finished(plea).map(|done| done.result),
        Some(QuestResult::Completed)
    );
    assert_eq!(sim.count(player, "tusk"), 0);
    let sided = sim.id::<MemoryDef>("sided");
    assert!(memory::get(&sim.world, player, sided).truthy());
    sim.leave(1);
    sim.run(0.5);
    assert!(!log(&sim, player).tracked.contains(&tusks));

    let friend = sim.talk(1, "shaman");
    assert_eq!(labels(&friend), vec!["Farewell.".to_owned()]);
}

/// A harbour of quest givers, each standing for one way quests are offered, followed and handed
/// in, with what they ask for at hand.
fn harbour() -> Sim {
    let quest_item =
        |kind: &str| json!({ "kind": { "type": kind }, "modules": [{ "type": "quest_item" }] });
    let content = Game::new()
        .area(
            "harbour",
            &Map::open(15, 15),
            json!([
                resident("giver", 2.0, 2.0),
                resident("captain", 7.0, 2.0),
                resident("keeper", 12.0, 2.0),
                resident("trader", 2.0, 12.0),
                resident("shaman", 7.0, 12.0),
                {
                    "type": "object",
                    "name": "stone",
                    "title": "Standing stone",
                    "shape": { "type": "point", "x": 12.0, "y": 12.0 },
                    "look": path("prop", "stone"),
                    "scripts": [{ "script": "stone_look.script.json" }],
                },
            ]),
        )
        .script(
            "stone_look",
            "object",
            json!({
                "interacted": { "type": "on", "event": "interacted", "verb": "use", "reach": 1.5, "next": "on_quest" },
                "on_quest": { "type": "if", "test": on_quest("x_marks"), "yes": "found", "no": "look" },
                "found": { "type": "complete_task", "quest": "x_marks.quest.json", "task": "find_stone", "next": "matches" },
                "matches": said("The stone matches the drawing.", "turn_in"),
                "turn_in": { "type": "turn_in", "quest": "x_marks.quest.json" },
                "look": dialogue("Moss covers the runes.", json!([{ "label": ["Step away."] }])),
            }),
        )
        .prop("stone", json!({ "type": "none" }))
        .hero("harbour", 7.0, 7.0)
        .edit("class", "hero", |hero| {
            hero["levels"] = json!([
                { "xp": 0, "skill_points": 1 },
                { "xp": 100 },
                { "xp": 200 },
            ])
        })
        .resource("gold", 100_000)
        .resource("bone", 50)
        .resource("token", 1000)
        .resource("tusk", 50)
        .resource("steak", 50)
        .item("pebble", json!({ "kind": { "type": "resource" } }))
        .item("shield", json!({ "kind": { "type": "equipment", "slot": "offhand" } }))
        .item("helm", json!({ "kind": { "type": "equipment", "slot": "head" } }))
        .item("cutlass", json!({ "kind": { "type": "equipment", "slot": "weapon" } }))
        .item("letter", quest_item("resource"))
        .item("key", quest_item("resource"))
        .item("map", {
            let mut map = quest_item("usable");
            map["modules"].as_array_mut().expect("modules").push(json!({ "type": "script", "script": "map_read.script.json" }));
            map
        })
        .script(
            "map_read",
            "item",
            json!({
                "used": { "type": "on", "event": "used", "next": "read" },
                "read": { "type": "if", "test": on_quest("x_marks"), "yes": "waiting", "no": "offer" },
                "offer": said("A cross is scrawled beside a standing stone.", "card"),
                "card": { "type": "offer_quest", "quest": "x_marks.quest.json" },
                "waiting": dialogue("The cross sits beside a standing stone.", json!([{ "label": ["Fold it away."] }])),
            }),
        )
        .npc("bat", 10.0, json!([]))
        .npc("rat", 10.0, json!([]))
        .script(
            "giver_talk",
            "npc",
            talker(
                json!([
                    { "label": ["A Letter"], "requires": not(quest_done("letter")), "to": "letter" },
                    { "label": ["Low Tide"], "requires": not(quest_done("low_tide")), "to": "low_tide" },
                    { "label": ["Goodbye."] },
                ]),
                [offered("letter", "I'll take it."), offered("low_tide", "I'll hurry.")],
            ),
        )
        .npc("giver", 40.0, json!([{ "type": "script", "script": "giver_talk.script.json" }]))
        .script(
            "captain_talk",
            "npc",
            talker(
                json!([
                    { "label": ["A Letter"], "requires": on_quest("letter"), "to": "letter_turn_in" },
                    {
                        "label": ["Bats"],
                        "requires": json!({ "type": "all", "of": [
                            not(quest_done("bats")),
                            { "type": "any", "of": [on_quest("bats"), bats_open()] },
                        ] }),
                        "to": "bats",
                    },
                    { "label": ["Goodbye."] },
                ]),
                [
                    json!({
                        "letter_turn_in": { "type": "turn_in", "quest": "letter.quest.json", "done": "letter_thanks" },
                        "letter_thanks": dialogue("A letter? For me?", json!([{ "label": ["Never mind."] }])),
                    }),
                    offered("bats", "I'll clear them."),
                ],
            ),
        )
        .npc("captain", 40.0, json!([{ "type": "script", "script": "captain_talk.script.json" }]))
        .script(
            "keeper_talk",
            "npc",
            talker(
                json!([
                    { "label": ["Tithe"], "requires": not(json!({ "type": "quest_cooling", "quest": "tithe.quest.json" })), "to": "tithe" },
                    { "label": ["Goodbye."] },
                ]),
                [offered("tithe", "I'll bring today's bones.")],
            ),
        )
        .npc("keeper", 40.0, json!([{ "type": "script", "script": "keeper_talk.script.json" }]))
        .script(
            "trader_talk",
            "npc",
            talker(
                json!([
                    { "label": ["Show me your wares."], "to": "wares" },
                    { "label": ["Tusks"], "requires": not(quest_done("tusks")), "to": "tusks" },
                    { "label": ["Goodbye."] },
                ]),
                [
                    json!({
                        "wares": said("Take your time.", "shop"),
                        "shop": { "type": "open_shop", "shop": "wares.shop.json" },
                    }),
                    offered("tusks", "I'll bring them."),
                ],
            ),
        )
        .npc("trader", 40.0, json!([{ "type": "script", "script": "trader_talk.script.json" }]))
        .npc("shaman", 40.0, json!([{ "type": "script", "script": "shaman_talk.script.json" }]))
        .script("shaman_talk", "npc", shaman())
        .row(
            "shop",
            "wares",
            json!({
                "title": "Wares",
                "mark": "merchant.attention.json",
                "modules": [{ "type": "buys", "what": { "type": "category", "category": "material" }, "pays": [{ "item": "gold.item.json" }] }],
            }),
        )
        .mark("merchant")
        .mark("quest_ready")
        .mark("quest_offered")
        .mark("repeatable_ready")
        .mark("repeatable_offered")
        .mark("quest_in_progress")
        .mark("quest_locked")
        .row(
            "quest",
            "letter",
            quest(
                "A Letter",
                json!([
                    { "type": "grants", "items": [{ "item": "letter.item.json" }] },
                    { "type": "deliver", "item": "letter.item.json" },
                    { "type": "hand_in", "items": [{ "item": "letter.item.json" }] },
                    { "type": "hand_over", "text": ["The giver asked me to bring you this."] },
                    { "type": "reward", "items": [{ "item": "gold.item.json", "count": 10 }] },
                ]),
            ),
        )
        .row(
            "quest",
            "bats",
            quest(
                "Bats",
                json!([
                    { "type": "requires", "condition": bats_open() },
                    { "type": "defeat", "npc": "bat.npc.json", "count": 4 },
                    { "type": "hold", "item": "key.item.json" },
                    { "type": "hand_in", "items": [{ "item": "key.item.json" }] },
                    { "type": "hand_over", "text": ["Here's your key."] },
                    { "type": "reward", "items": [{ "item": "gold.item.json", "count": 25 }] },
                    { "type": "drop", "from": "bat.npc.json", "item": "key.item.json", "chance": 10.0, "grows": 15.0 },
                ]),
            ),
        )
        .row(
            "quest",
            "tusks",
            quest(
                "Tusks",
                json!([
                    { "type": "shown", "condition": not(json!({ "type": "memory", "memory": "sided.memory.json" })) },
                    { "type": "hold", "item": "tusk.item.json", "count": 5 },
                    { "type": "hand_in", "items": [{ "item": "tusk.item.json", "count": 5 }] },
                    { "type": "reward", "items": [{ "item": "gold.item.json", "count": 35 }] },
                    { "type": "pick_one", "items": [{ "item": "shield.item.json" }, { "item": "helm.item.json" }] },
                ]),
            ),
        )
        .row(
            "quest",
            "tithe",
            quest(
                "Tithe",
                json!([
                    { "type": "hold", "item": "bone.item.json", "count": 10 },
                    { "type": "hand_in", "items": [{ "item": "bone.item.json", "count": 10 }] },
                    { "type": "hand_over", "text": ["Here are today's bones."] },
                    { "type": "reward", "items": [{ "item": "token.item.json", "count": 6 }] },
                    { "type": "daily" },
                ]),
            ),
        )
        .row(
            "quest",
            "low_tide",
            quest(
                "Low Tide",
                json!([
                    { "type": "hold", "item": "steak.item.json", "count": 3 },
                    { "type": "hand_in", "items": [{ "item": "steak.item.json", "count": 3 }] },
                    { "type": "hand_over", "text": ["Here are the steaks."] },
                    { "type": "reward", "items": [{ "item": "gold.item.json", "count": 15 }] },
                    { "type": "time_limit", "seconds": 600.0 },
                ]),
            ),
        )
        .row(
            "quest",
            "x_marks",
            quest(
                "X Marks the Spot",
                json!([
                    { "type": "task", "task": "find_stone", "label": "Find the standing stone" },
                    { "type": "hand_in", "items": [{ "item": "map.item.json" }] },
                    { "type": "hand_over", "text": ["Dig where the cross says."] },
                    { "type": "reward", "items": [{ "item": "cutlass.item.json" }] },
                ]),
            ),
        )
        .row(
            "quest",
            "plea",
            quest(
                "The Plea",
                json!([
                    { "type": "shown", "condition": on_quest("tusks") },
                    {
                        "type": "decide",
                        "label": "Answer the shaman",
                        "paths": [
                            { "choice": "Give the tusks", "means": ["Tusks fails"] },
                            { "choice": "Keep them", "means": ["Tusks continues"] },
                        ],
                    },
                ]),
            ),
        )
        .memory("sided", "never")
        .memory("spurned", "never")
        .vfx("damage_taken")
        .shader("damage_pop")
        .vfx("defeated")
        .sfx("ui_refuse")
        .sfx("ui_close")
        .sfx("coins")
        .sfx("quest_accepted")
        .sfx("quest_completed")
        .sfx("quest_abandoned")
        .sfx("ui_chime")
        .sfx("tally_tick")
        .sfx("ui_page")
        .load();
    let mut sim = Sim::new(content);
    sim.serve("harbour")
        .accept::<InteractRequest>()
        .accept::<ConversationRequest>()
        .accept::<ShopRequest>()
        .accept::<QuestRequest>()
        .accept::<UseItemRequest>()
        .accept::<DropItemRequest>()
        .add((
            interact::interact_request,
            item::use_item,
            item::drop_item,
            dialogue::requests,
            shop::requests,
            quest::requests,
        ))
        .add((rewards::grant, quest::credit, npc::defeated))
        .add((
            script::run_tick,
            movement::advance,
            interact::interactions,
            dialogue::hold,
            dialogue::refresh,
            shop::hold,
            shop::refresh,
            quest::refresh,
        ))
        .add((stat::settle_health, combat::defeats));
    script::spawn_stateless(&mut sim.world);
    npc::spawn_all(&mut sim.world);
    area::object::spawn_all(&mut sim.world);
    sim
}

fn resident(npc: &str, x: f32, y: f32) -> Value {
    json!({ "type": "resident", "npc": path("npc", npc), "at": { "x": x, "y": y } })
}

fn talker<const N: usize>(choices: Value, topics: [Value; N]) -> Value {
    let mut nodes = json!({
        "interacted": { "type": "on", "event": "interacted", "verb": "talk", "reach": 2.0, "next": "hello" },
        "hello": dialogue("Well met.", choices),
    });
    for topic in topics {
        for (key, node) in topic.as_object().expect("nodes") {
            nodes[key] = node.clone();
        }
    }
    nodes
}

/// The nodes of a quest offered on its title's choice, accepted with `accept`, and handed in on
/// the same choice once on it.
fn offered(quest: &str, accept: &str) -> Value {
    let mut nodes = serde_json::Map::new();
    let node = |suffix: &str| format!("{quest}_{suffix}");
    nodes.insert(
        quest.to_owned(),
        json!({ "type": "if", "test": on_quest(quest), "yes": node("turn_in"), "no": node("offer") }),
    );
    nodes.insert(
        node("offer"),
        dialogue(
            "Would you help?",
            json!([{ "label": [accept], "to": node("accept") }, { "label": ["Not now."] }]),
        ),
    );
    nodes.insert(
        node("accept"),
        json!({ "type": "accept_quest", "quest": path("quest", quest) }),
    );
    nodes.insert(
        node("turn_in"),
        json!({ "type": "turn_in", "quest": path("quest", quest), "done": node("thanks"), "refused": node("waiting") }),
    );
    nodes.insert(
        node("thanks"),
        dialogue("Thank you.", json!([{ "label": ["Goodbye."] }])),
    );
    nodes.insert(
        node("waiting"),
        dialogue("Not yet, then.", json!([{ "label": ["Right."] }])),
    );
    Value::Object(nodes)
}

/// A shaman who asks for the tusks the trader wants, and befriends whoever hands them over.
fn shaman() -> Value {
    json!({
            "interacted": { "type": "on", "event": "interacted", "verb": "talk", "reach": 2.0, "next": "befriended" },
            "befriended": { "type": "if", "test": { "type": "memory", "memory": "sided.memory.json" }, "yes": "friend", "no": "spurned_before" },
            "spurned_before": { "type": "if", "test": { "type": "memory", "memory": "spurned.memory.json" }, "yes": "silent", "no": "hello" },
            "silent": dialogue("She turns away.", json!([{ "label": ["Leave."] }])),
            "hello": dialogue("Speak, outlander.", json!([
                {
                    "label": ["The Plea"],
                    "requires": { "type": "all", "of": [on_quest("tusks"), not(quest_done("plea"))] },
                    "to": "plea",
                },
                { "label": ["I'll leave you be."] },
            ])),
            "plea": { "type": "if", "test": on_quest("plea"), "yes": "plea_answer", "no": "plea_offer" },
            "plea_offer": dialogue("Those tusks were cut from our dead.", json!([
                { "label": ["I'm listening."], "to": "plea_accept" },
                { "label": ["Not now."] },
            ])),
            "plea_accept": { "type": "accept_quest", "quest": "plea.quest.json", "done": "plea_answer" },
            "plea_answer": dialogue("Give me the tusks of my kin.", json!([
                { "label": ["Give her the tusks."], "requires": on_quest("tusks"), "to": "give_tusks" },
                { "label": ["Keep them."], "to": "keep_tusks" },
                { "label": ["Let me think on it."] },
            ])),
            "give_tusks": { "type": "items", "take": [{ "item": "tusk.item.json", "count": 5 }], "done": "side_with_clan" },
            "side_with_clan": { "type": "decide", "quest": "plea.quest.json", "path": "Give the tusks", "next": "plea_given" },
            "plea_given": { "type": "turn_in", "quest": "plea.quest.json", "done": "fail_tusks" },
            "fail_tusks": { "type": "fail_quest", "quest": "tusks.quest.json", "next": "remember_sided" },
            "remember_sided": { "type": "remember", "memory": "sided.memory.json", "next": "grateful" },
            "grateful": dialogue("Then the clan owes you.", json!([{ "label": ["Rest easy."] }])),
            "keep_tusks": { "type": "decide", "quest": "plea.quest.json", "path": "Keep them", "next": "plea_kept" },
            "plea_kept": { "type": "turn_in", "quest": "plea.quest.json", "done": "remember_spurned" },
            "remember_spurned": { "type": "remember", "memory": "spurned.memory.json", "next": "silent" },
            "friend": dialogue("Friend of the clan.", json!([{ "label": ["Farewell."] }])),
    })
}

fn quest(title: &str, modules: Value) -> Value {
    json!({ "title": title, "category": "Errands", "blurb": title, "xp": 10, "modules": modules })
}

fn bats_open() -> Value {
    json!({ "type": "all", "of": [
        { "type": "at_least", "a": { "type": "level" }, "b": 3 },
        quest_done("letter"),
    ] })
}

fn on_quest(quest: &str) -> Value {
    json!({ "type": "on_quest", "quest": path("quest", quest) })
}

fn quest_done(quest: &str) -> Value {
    json!({ "type": "quest_done", "quest": path("quest", quest) })
}

fn not(condition: Value) -> Value {
    json!({ "type": "not", "of": condition })
}

fn accept(sim: &mut Sim, player: Entity, key: &str) {
    let quest = sim.id::<QuestDef>(key);
    quest::accept(&mut sim.world, player, quest).expect("accepted");
}

fn log(sim: &Sim, player: Entity) -> QuestLog {
    sim.world
        .get::<QuestLog>(player)
        .cloned()
        .expect("a quest log")
}

fn level_up(sim: &mut Sim, player: Entity) {
    sim.world.get_mut::<Xp>(player).expect("xp").gain(100_000);
}

fn deliver_letter(sim: &mut Sim, player: Entity) {
    accept(sim, player, "letter");
    let letter = sim.id::<QuestDef>("letter");
    quest::turn_in(&mut sim.world, player, letter, None).expect("delivered");
}

fn qualify_for_bats(sim: &mut Sim, player: Entity) {
    level_up(sim, player);
    deliver_letter(sim, player);
}

/// A character of the npc row under `key`, struck down at `player`'s feet with the kill
/// reserved for them.
fn slay(sim: &mut Sim, player: Entity, key: &str) {
    let at = movement::position(&sim.world, player).expect("player position");
    let area = sim.world.resource::<engine::systems::WorldArea>().0;
    let def = sim.id::<npc::NpcDef>(key);
    let victim = npc::spawn(&mut sim.world, def, at, area, npc::Pack(u32::MAX));
    item::reserve(
        &mut sim.world,
        victim,
        player,
        engine::core::time::Seconds(0.0),
    );
    combat::inflict(&mut sim.world, victim, Some(player), 1_000_000.0);
    sim.run(0.2);
}

fn fill_bag(sim: &mut Sim, player: Entity) {
    let used = sim
        .world
        .get::<Inventory>(player)
        .expect("a bag")
        .slots
        .len();
    for _ in used..INVENTORY_MAX as usize {
        sim.give(player, "pebble", 1);
    }
}
