use std::collections::BTreeMap;

use engine::core::content::{Content, ContentFile, ContentId};
use engine::core::math::Pos;
use engine::core::script::Value;
use engine::core::time::{Seconds, UnixMillis, UtcHour, WallClock};
use engine::systems::actor::stance::HandIndex;
use engine::systems::actor::{Appearance, Name};
use engine::systems::area::AreaDef;
use engine::systems::area::transition::Discovered;
use engine::systems::class::{Class, ClassDef};
use engine::systems::equipment::{Equipment, EquipmentPlace};
use engine::systems::history::{History, HistoryEntry, HistoryTopic};
use engine::systems::item::{Inventory, ItemDef, ItemStack};
use engine::systems::memory::{self, MemoryDef};
use engine::systems::movement::Position;
use engine::systems::player::snapshot::{
    AtInitialSpawn, CharacterSnapshot, CharacterStart, HeardLine, JoinAs, MemoryValue, QuestState,
    SnapshotMemory, SnapshotQuest, SnapshotRecord, SnapshotShop,
};
use engine::systems::player::{SpawnPolicy, SpawnRules, Xp};
use engine::systems::quest::{QuestDef, QuestLog, QuestResult};
use engine::systems::script::Place;
use engine::systems::shop::{Sale, ShopDef, ShopLedger};
use engine::systems::stat::{StatKind, Stats};
use engine::systems::text::LineText;
use engine::systems::{GameSettings, dialogue};
use serde_json::json;
use testkit::{Declared, Game, Map, Sim};

#[test]
fn a_character_joined_from_a_row_is_that_character() {
    let mut sim = returning_to("forest");
    let content = sim.world.resource::<Content>().clone();
    let forest = sim.id::<AreaDef>("forest");
    let start = Place {
        area: forest,
        at: Pos::new(6.0, 4.0),
    };
    let snapshot = tess(&content, start);

    let player = sim.join_as(
        1,
        JoinAs {
            snapshot: snapshot.clone(),
            start,
        },
    );
    let world = &sim.world;

    assert_eq!(
        world.get::<Name>(player).map(|name| name.name.as_str()),
        Some("Tess")
    );
    let landed = world
        .get::<Position>(player)
        .map(|position| position.pos)
        .expect("a position");
    assert!(
        landed.distance_to(start.at) <= 1.5,
        "landed at {landed:?}, asked for {:?}",
        start.at
    );
    assert_eq!(world.get::<Xp>(player), Some(&Xp { amount: 60 }));
    assert_eq!(
        world.get::<Class>(player),
        Some(&Class {
            def: id::<ClassDef>(&content, "hero")
        })
    );
    let stats = world.get::<Stats>(player).expect("stats");
    assert_eq!(stats.get(StatKind::Damage), 9.0);
    assert_eq!(stats.get(StatKind::MaxHealth), 30.0);
    assert_eq!(
        world.get::<Inventory>(player).map(|bag| bag.slots.clone()),
        Some(snapshot.bag.clone())
    );
    assert_eq!(
        world.get::<Equipment>(player),
        Some(&Equipment {
            slots: BTreeMap::from([(EquipmentPlace::Hand(HandIndex::OFF), id(&content, "shield"))])
        })
    );
    let known = world.get::<Discovered>(player).expect("known areas");
    assert!(known.knows(forest) && known.knows(id::<AreaDef>(&content, "village")));
    let joined_at = world.resource::<WallClock>().now;
    let log = world.get::<QuestLog>(player).expect("quest log");
    let errand = id::<QuestDef>(&content, "errand");
    assert_eq!(
        log.active(errand).expect("on the errand").counts,
        vec![0, 1]
    );
    assert_eq!(
        log.finished(id::<QuestDef>(&content, "letter"))
            .map(|finished| finished.result),
        Some(QuestResult::Completed)
    );
    assert_eq!(log.tracked, vec![errand]);
    let taunted = id::<MemoryDef>(&content, "taunted");
    assert_eq!(memory::get(world, player, taunted), Value::Bool(true));
    assert!(memory::set_within(world, player, taunted, Seconds(20.0)));
    assert!(!memory::set_within(world, player, taunted, Seconds(5.0)));
    assert!(dialogue::heard(world, player, "npc/crier", "news"));
    let mut ledger = ShopLedger::default();
    ledger.record_sale(id::<ShopDef>(&content, "wares"), sale(&content));
    assert_eq!(world.get::<ShopLedger>(player), Some(&ledger));
    let history = world.get::<History>(player).expect("history");
    let records: Vec<_> = history.records().collect();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].entry.text, LineText::plain("Hello"));
    assert_eq!(records[0].at, joined_at.before(Seconds(5.0)));
}

#[test]
fn a_start_of_its_own_moves_the_character_and_nothing_else() {
    let mut sim = returning_to("village");
    let content = sim.world.resource::<Content>().clone();
    let village = sim.id::<AreaDef>("village");
    let spawn = content.built::<GameSettings>().initial_spawn;
    let snapshot = CharacterSnapshot {
        start: CharacterStart::InitialSpawn(AtInitialSpawn::InitialSpawn),
        ..tess(&content, spawn)
    };
    let here = Place {
        area: village,
        at: Pos::new(spawn.at.x + 3.0, spawn.at.y),
    };

    let player = sim.join_as(
        1,
        JoinAs {
            snapshot: snapshot.clone(),
            start: here,
        },
    );
    let world = &sim.world;

    assert_eq!(
        world.get::<Position>(player).map(|position| position.pos),
        Some(here.at)
    );
    assert_eq!(world.get::<Xp>(player), Some(&Xp { amount: 60 }));
    assert!(
        world
            .get::<Discovered>(player)
            .expect("known areas")
            .knows(village)
    );
}

// A snapshot counts its times back from now, which needs a clock well past zero, and its stats
// hold only for a mortal character.
fn returning_to(area: &str) -> Sim {
    let content = Game::new()
        .area("village", &Map::open(9, 9), json!([]))
        .area("forest", &Map::open(9, 9), json!([]))
        .hero("village", 4.0, 4.0)
        .item(
            "gold",
            json!({ "kind": { "type": "resource" }, "modules": [{ "type": "stack", "max": 1000 }] }),
        )
        .item(
            "tusk",
            json!({ "kind": { "type": "resource" }, "modules": [{ "type": "stack", "max": 50 }] }),
        )
        .item("bone", json!({ "kind": { "type": "resource" } }))
        .stance("shield", 1, "off")
        .item("shield", json!({ "kind": { "type": "equipment", "wear": "held", "stance": "shield.stance.json" } }))
        .npc("chief", 60.0, json!([]))
        .row(
            "quest",
            "errand",
            json!({
                "title": "An errand",
                "category": "Errands",
                "blurb": "Bring five tusks, and the chief down.",
                "xp": 120,
                "modules": [
                    { "type": "hold", "item": "tusk.item.json", "count": 5 },
                    { "type": "defeat", "npc": "chief.npc.json", "count": 1 },
                    { "type": "hand_over", "text": ["Done."] },
                ],
            }),
        )
        .row(
            "quest",
            "letter",
            json!({
                "title": "A letter",
                "category": "Errands",
                "blurb": "Hold on to this.",
                "xp": 40,
                "modules": [
                    { "type": "hold", "item": "bone.item.json", "count": 1 },
                    { "type": "hand_over", "text": ["Here."] },
                ],
            }),
        )
        .row(
            "shop",
            "wares",
            json!({
                "title": "Wares",
                "mark": "merchant.attention.json",
                "modules": [{
                    "type": "sells",
                    "item": "bone.item.json",
                    "price": [{ "item": "gold.item.json", "count": 2 }],
                    "stock": { "type": "unlimited" },
                }],
            }),
        )
        .mark("merchant")
        .memory("taunted", "never")
        .load();
    let mut sim = Sim::new(content);
    sim.serve(area);
    sim.world.insert_resource(WallClock {
        now: UnixMillis(1_000_000_000),
        reset: UtcHour::MIDNIGHT,
    });
    sim.world.insert_resource(SpawnRules {
        policy: SpawnPolicy::Map,
        immortal: false,
    });
    sim
}

fn tess(content: &Content, start: Place) -> CharacterSnapshot {
    CharacterSnapshot {
        name: "Tess".to_owned(),
        class: id(content, "hero"),
        appearance: Appearance::of(id(content, "hero")),
        xp: 60,
        skills: Vec::new(),
        hotbar: Vec::new(),
        basic_attack: None,
        stats: Stats(BTreeMap::from([(StatKind::Damage, 9.0)])),
        bag: vec![
            ItemStack::new(id(content, "tusk"), 5),
            ItemStack::new(id(content, "gold"), 20),
        ],
        equipped: vec![id(content, "shield")],
        known_areas: vec![id(content, "village"), id(content, "forest")],
        quests: vec![
            SnapshotQuest {
                quest: id(content, "errand"),
                state: QuestState::Active,
                progress: vec![0, 1],
                left: None,
                decided: None,
            },
            SnapshotQuest {
                quest: id(content, "letter"),
                state: QuestState::Completed,
                progress: Vec::new(),
                left: None,
                decided: None,
            },
        ],
        memories: vec![SnapshotMemory {
            memory: id(content, "taunted"),
            value: MemoryValue::Bool(true),
            ago: Seconds(10.0),
        }],
        lines_heard: vec![HeardLine {
            script: "npc/crier".to_owned(),
            node: "news".to_owned(),
        }],
        shops: vec![SnapshotShop {
            shop: id(content, "wares"),
            stock: Vec::new(),
            sales: vec![sale(content)],
        }],
        history: vec![SnapshotRecord {
            ago: Seconds(5.0),
            entry: HistoryEntry::of(HistoryTopic::Talk, LineText::plain("Hello")),
        }],
        start: CharacterStart::At(start),
    }
}

fn sale(content: &Content) -> Sale {
    Sale {
        item: id::<ItemDef>(content, "bone"),
        count: 2,
        paid: vec![ItemStack::new(id(content, "gold"), 4)],
    }
}

fn id<Row: ContentFile>(content: &Content, key: &str) -> ContentId<Row> {
    content
        .declared::<Row>(key)
        .unwrap_or_else(|| panic!("no {} row `{key}`", Row::TABLE))
}
