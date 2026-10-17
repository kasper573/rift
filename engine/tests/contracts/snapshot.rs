use std::collections::BTreeMap;

use engine::core::math::Pos;
use engine::core::script::Value;
use engine::core::time::{Seconds, UnixMillis, WallClock};
use engine::systems::GameSettings;
use engine::systems::actor::Name;
use engine::systems::area::AreaId;
use engine::systems::area::transition::Discovered;
use engine::systems::class::Class;
use engine::systems::dialogue;
use engine::systems::equipment::{Equipment, EquipmentSlot};
use engine::systems::history::{History, HistoryEntry, HistoryTopic};
use engine::systems::item::{Inventory, ItemStack};
use engine::systems::memory;
use engine::systems::movement::Position;
use engine::systems::player::snapshot::{
    AtInitialSpawn, CharacterSnapshot, CharacterStart, HeardLine, JoinAs, MemoryValue, QuestState,
    SnapshotMemory, SnapshotQuest, SnapshotRecord, SnapshotShop,
};
use engine::systems::player::{SpawnPolicy, SpawnRules, Xp};
use engine::systems::quest::{QuestLog, QuestResult};
use engine::systems::script::Place;
use engine::systems::shop::{Sale, ShopLedger};
use engine::systems::stat::{StatKind, StatMap, Stats};
use engine::systems::text::LineText;

use testkit::{CLOCK, Sim, content, row};

const LATER: WallClock = WallClock {
    now: UnixMillis(1_000_000_000),
    ..CLOCK
};

#[test]
fn a_character_joined_from_a_row_is_that_character() {
    let forest = row("forest");
    let start = Place {
        area: forest,
        at: Pos::new(12.0, 10.0),
    };
    let snapshot = tess(start);
    let mut sim = returning_to(forest);

    let player = sim.join_as(
        1,
        JoinAs {
            snapshot: snapshot.clone(),
            start,
        },
    );
    let world = sim.world();

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
            def: row("fighter")
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
            slots: snapshot.equipment.clone()
        })
    );
    let known = world.get::<Discovered>(player).expect("known areas");
    assert!(known.knows(forest) && known.knows(row("island")));
    let joined_at = world.resource::<WallClock>().now;
    let log = world.get::<QuestLog>(player).expect("quest log");
    let tusks = log
        .active(row("tusks_for_the_chief"))
        .expect("on the errand");
    assert_eq!(tusks.counts, vec![0, 1]);
    assert_eq!(
        log.finished(row("letter_for_the_captain"))
            .map(|finished| finished.result),
        Some(QuestResult::Completed)
    );
    assert_eq!(log.tracked, vec![row("tusks_for_the_chief")]);
    assert_eq!(
        memory::get(world, player, row("chief_taunted_you")),
        Value::Bool(true)
    );
    assert!(memory::set_within(
        world,
        player,
        row("chief_taunted_you"),
        Seconds(20.0)
    ));
    assert!(!memory::set_within(
        world,
        player,
        row("chief_taunted_you"),
        Seconds(5.0)
    ));
    assert!(dialogue::heard(world, player, "npc/tobb", "news"));
    let mut ledger = ShopLedger::default();
    ledger.record_sale(row("mara_wares"), sale());
    assert_eq!(world.get::<ShopLedger>(player), Some(&ledger));
    let history = world.get::<History>(player).expect("history");
    let records: Vec<_> = history.records().collect();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].entry.text, LineText::plain("Hello"));
    assert_eq!(records[0].at, joined_at.before(Seconds(5.0)));
}

#[test]
fn a_start_of_its_own_moves_the_character_and_nothing_else() {
    let island = row("island");
    let spawn = content().built::<GameSettings>().initial_spawn;
    let snapshot = CharacterSnapshot {
        start: CharacterStart::InitialSpawn(AtInitialSpawn::InitialSpawn),
        ..tess(spawn)
    };
    let here = Place {
        area: island,
        at: Pos::new(spawn.at.x + 3.0, spawn.at.y),
    };
    let mut sim = returning_to(island);

    let player = sim.join_as(
        1,
        JoinAs {
            snapshot: snapshot.clone(),
            start: here,
        },
    );
    let world = sim.world();

    assert_eq!(
        world.get::<Position>(player).map(|position| position.pos),
        Some(here.at)
    );
    assert_eq!(world.get::<Xp>(player), Some(&Xp { amount: 60 }));
    assert!(
        world
            .get::<Discovered>(player)
            .expect("known areas")
            .knows(island)
    );
}

// A snapshot counts its times back from now, which needs a clock well past zero, and its stats
// hold only for a mortal character.
fn returning_to(area: AreaId) -> Sim {
    let mut sim = Sim::area(area);
    sim.world().insert_resource(LATER);
    sim.world().insert_resource(SpawnRules {
        policy: SpawnPolicy::Map,
        immortal: false,
    });
    sim
}

fn tess(start: Place) -> CharacterSnapshot {
    CharacterSnapshot {
        name: "Tess".to_owned(),
        class: row("fighter"),
        xp: 60,
        skills: Vec::new(),
        hotbar: Vec::new(),
        basic_attack: None,
        stats: StatMap(BTreeMap::from([(StatKind::Damage, 9.0)])),
        bag: vec![
            ItemStack::new(row("orc_tusk"), 5),
            ItemStack::new(row("gold"), 20),
        ],
        equipment: BTreeMap::from([(EquipmentSlot::Offhand, row("bone_shield"))]),
        known_areas: vec![row("island"), row("forest")],
        quests: vec![
            SnapshotQuest {
                quest: row("tusks_for_the_chief"),
                state: QuestState::Active,
                progress: vec![0, 1],
                left: None,
                decided: None,
            },
            SnapshotQuest {
                quest: row("letter_for_the_captain"),
                state: QuestState::Completed,
                progress: Vec::new(),
                left: None,
                decided: None,
            },
        ],
        memories: vec![SnapshotMemory {
            memory: row("chief_taunted_you"),
            value: MemoryValue::Bool(true),
            ago: Seconds(10.0),
        }],
        lines_heard: vec![HeardLine {
            script: "npc/tobb".to_owned(),
            node: "news".to_owned(),
        }],
        shops: vec![SnapshotShop {
            shop: row("mara_wares"),
            stock: Vec::new(),
            sales: vec![sale()],
        }],
        history: vec![SnapshotRecord {
            ago: Seconds(5.0),
            entry: HistoryEntry::of(HistoryTopic::Talk, LineText::plain("Hello")),
        }],
        start: CharacterStart::At(start),
    }
}

fn sale() -> Sale {
    Sale {
        item: row("bone"),
        count: 2,
        paid: vec![ItemStack::new(row("gold"), 4)],
    }
}
