use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use engine::core::content::{
    Content, ContentFile, ContentId, ContentScope, SavedList, with_content,
};
use engine::core::math::Pos;
use engine::core::script::Value;
use engine::core::text::LineText;
use engine::core::time::{UnixMillis, UtcHour, WallClock};
use engine::core::units::Seconds;
use engine::systems::actor::stance::HandIndex;
use engine::systems::actor::{Appearance, Name};
use engine::systems::area::AreaDef;
use engine::systems::area::Place;
use engine::systems::area::transition::Discovered;
use engine::systems::class::{Class, ClassDef, NodeKey};
use engine::systems::command::{CommandCooldowns, TerminalCommandDef};
use engine::systems::equipment::{Equipment, EquipmentPlace};
use engine::systems::history::{History, HistoryEntry, HistoryTopic};
use engine::systems::item::{Inventory, ItemDef, ItemStack};
use engine::systems::memory::{self, MemoryDef};
use engine::systems::movement::Position;
use engine::systems::player::snapshot::{
    AtInitialSpawn, BagEntry, CharacterSnapshot, CharacterStart, HeardLine, JoinAs, JoinOrigin,
    MemoryValue, QuestState, RunningStatus, ShopStock, SnapshotBeat, SnapshotCommandCooldown,
    SnapshotCooldown, SnapshotInstanceCooldown, SnapshotMemory, SnapshotQuest, SnapshotRecord,
    SnapshotShop, SnapshotSkill, SnapshotSlot, SnapshotStatus,
};
use engine::systems::player::{CharacterState, SpawnPoint, SpawnPolicy, SpawnRules, Xp};
use engine::systems::quest::{QuestDef, QuestLog, QuestResult};
use engine::systems::shop::{Sale, ShopDef, ShopLedger};
use engine::systems::skill::{
    self, BasicAttack, Cooldowns, HotbarEntry, HotbarSlot, Rank, SkillDef,
};
use engine::systems::stat::{self, StatKind, Stats};
use engine::systems::status::{self, StatusEffectDef, Statuses};
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
            origin: JoinOrigin::Staged,
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
    let guard = id::<SkillDef>(&content, "guard");
    assert_eq!(
        skill::known(world, player).get(&guard),
        Some(&Rank::FIRST),
        "the rank it learned"
    );
    assert_eq!(
        world.get::<BasicAttack>(player),
        Some(&BasicAttack(Some(id(&content, "punch"))))
    );
    assert_eq!(
        world.get::<Inventory>(player).map(|bag| bag
            .stacks()
            .map(|(slot, stack)| BagEntry::of(slot as u32, stack))
            .collect::<Vec<_>>()),
        Some(snapshot.bag.to_vec()),
        "every stack in the slot it was saved in"
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
    let wares = id::<ShopDef>(&content, "wares");
    ledger.record_sale(wares, sale(&content), wares.get(&content).buys_back);
    ledger.stocked(wares, 1, 2, joined_at.after(Seconds(400.0)));
    assert_eq!(world.get::<ShopLedger>(player), Some(&ledger));
    let history = world.get::<History>(player).expect("history");
    let records: Vec<_> = history.records().collect();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].entry.text, hello());
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
            origin: JoinOrigin::Staged,
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

#[test]
fn a_character_keeps_everything_across_a_warp_and_across_being_saved() {
    let mut village = returning_to("village");
    let mut forest = returning_to("forest");
    let content = village.world.resource::<Content>().clone();
    let start = content.built::<GameSettings>().initial_spawn;
    let player = village.join_as(
        1,
        JoinAs {
            snapshot: tess(&content, start),
            start,
            origin: JoinOrigin::Staged,
        },
    );
    let blessed = village.id::<StatusEffectDef>("blessed");
    let guard = village.id::<SkillDef>("guard");
    let unblessed = stat::max_health(&village.world, player);
    status::apply(
        &mut village.world,
        player,
        blessed,
        7.0,
        Some(Seconds(600.0)),
    );
    village.run(1.5);
    assert_eq!(village.count(player, "mark"), 1, "applied");
    assert_eq!(village.count(player, "beat"), 0, "its interval not yet up");

    let landing = Pos::new(5.0, 5.0);
    let arrived = village.cross(player, &mut forest, landing);
    forest
        .world
        .insert_resource(*village.world.resource::<WallClock>());
    forest.run(1.0);
    assert!(
        status::has(&forest.world, arrived, blessed),
        "an effect crosses a warp"
    );
    assert_eq!(
        forest.count(arrived, "mark"),
        1,
        "and is not applied again there"
    );
    assert_eq!(
        stat::max_health(&forest.world, arrived),
        unblessed + 7.0,
        "what it holds is held on"
    );
    assert_eq!(
        forest.count(arrived, "beat"),
        1,
        "its interval kept its beat"
    );
    let cooling = |sim: &Sim, character: Entity| {
        let now = sim.world.resource::<WallClock>().now;
        sim.world
            .get::<Cooldowns>(character)
            .is_some_and(|cooldowns| cooldowns.cooling(guard, now))
    };
    assert!(cooling(&forest, arrived), "a cooldown crosses a warp");
    skill::reset::reset(&mut forest.world, arrived);
    assert!(cooling(&forest, arrived), "and outlasts a reset of skills");

    let live = state(&forest, arrived);
    assert!(
        live.history
            .records()
            .any(|record| record.entry.text == hello()),
        "history crosses a warp"
    );
    let snapshot = CharacterSnapshot::of_live(&forest.world, arrived).expect("a character");
    let scope = ContentScope::of(&content);
    let (written, _) = with_content(scope.clone(), || serde_json::to_string(&snapshot));
    let written = written.expect("a snapshot is written");
    let (read, _) = with_content(scope, || {
        serde_json::from_str::<CharacterSnapshot>(&written)
    });
    let read = read.expect("and read back");
    assert!(read.history.is_empty(), "history is never saved");
    assert_eq!(
        read,
        CharacterSnapshot {
            history: Vec::new(),
            ..snapshot
        }
    );
    let CharacterStart::At(place) = read.start else {
        panic!(
            "a live character is saved where it stands: {:?}",
            read.start
        );
    };
    assert_eq!(place.area, forest.id::<AreaDef>("forest"));

    let mut again = returning_to("forest");
    let clock = *forest.world.resource::<WallClock>();
    again.world.insert_resource(clock);
    let loaded = again.join_as(
        1,
        JoinAs {
            snapshot: read,
            start: place,
            origin: JoinOrigin::Saved {
                at: clock.now,
                told: Vec::new(),
            },
        },
    );
    again.tick();
    let loaded_state = state(&again, loaded);
    assert!(
        !loaded_state
            .history
            .records()
            .any(|record| record.entry.text == hello()),
        "and is gone once the character comes back from being saved"
    );
    assert_eq!(
        loaded_state,
        CharacterState {
            history: loaded_state.history.clone(),
            ..live
        }
    );
    assert_eq!(
        stat::max_health(&again.world, loaded),
        unblessed + 7.0,
        "what the effect holds is held on"
    );
    assert_eq!(
        again.count(loaded, "mark"),
        1,
        "and it is not applied again"
    );
    let stands = again.world.get::<Position>(loaded).expect("a position").pos;
    assert!(stands.distance_to(place.at) <= 1.5, "stands at {stands:?}");
}

#[test]
fn what_runs_on_the_clock_ran_while_a_saved_character_was_away_and_its_effects_waited() {
    let mut forest = returning_to("forest");
    let content = forest.world.resource::<Content>().clone();
    let start = Place {
        area: forest.id::<AreaDef>("forest"),
        at: Pos::new(4.0, 4.0),
    };
    let blessed = forest.id::<StatusEffectDef>("blessed");
    let guard = forest.id::<SkillDef>("guard");
    let snapshot = CharacterSnapshot {
        statuses: vec![SnapshotStatus {
            effect: blessed,
            power: 7.0,
            ago: Seconds(30.0),
            left: Some(Seconds(60.0)),
            running: Some(RunningStatus {
                beats: vec![SnapshotBeat {
                    every: Seconds(2.0),
                    next_in: Seconds(1.0),
                }],
                boosts: Vec::new(),
            }),
        }]
        .into(),
        ..tess(&content, start)
    };
    let now = forest.world.resource::<WallClock>().now;
    let away = Seconds(250.0);

    let back = forest.join_as(
        1,
        JoinAs {
            snapshot,
            start,
            origin: JoinOrigin::Saved {
                at: now.before(away),
                told: Vec::new(),
            },
        },
    );
    forest.run(1.5);
    let world = &forest.world;

    let ready = world
        .get::<Cooldowns>(back)
        .and_then(|cooldowns| cooldowns.ready_at(guard))
        .expect("a cooldown");
    let wave = forest.id::<TerminalCommandDef>("wave");
    let command_left = world
        .get::<CommandCooldowns>(back)
        .and_then(|cooldowns| cooldowns.left(wave, now));
    assert_eq!(
        (ready, command_left),
        (now.after(Seconds(50.0)), Some(Seconds(50.0))),
        "250 of its 300 seconds are served, a skill's and a command's alike"
    );
    let log = world.get::<QuestLog>(back).expect("quest log");
    let errand = log
        .active(forest.id::<QuestDef>("errand"))
        .expect("on the errand");
    assert_eq!(errand.deadline, Some(now.after(Seconds(650.0))));
    assert!(!memory::set_within(
        world,
        back,
        forest.id::<MemoryDef>("taunted"),
        Seconds(200.0)
    ));
    let held = world
        .get::<Statuses>(back)
        .and_then(|statuses| statuses.get(blessed).copied())
        .expect("the effect");
    assert_eq!(
        held.until,
        Some(now.after(Seconds(60.0))),
        "it has all it had left"
    );
    assert_eq!(
        forest.count(back, "beat"),
        1,
        "and bites when it would have"
    );
    assert_eq!(forest.count(back, "mark"), 0, "without being applied again");
}

/// Everything `character` is, as it would cross to another world.
fn state(sim: &Sim, character: Entity) -> CharacterState {
    let rises = sim
        .world
        .get::<SpawnPoint>(character)
        .expect("a spawn point");
    CharacterState::of(&sim.world, character, rises.0).expect("a character")
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
        .quest("errand",
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
        .quest("letter",
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
                "buys_back": 3,
                "modules": [
                    {
                        "type": "sells",
                        "item": "bone.item.json",
                        "price": [{ "item": "gold.item.json", "count": 2 }],
                        "stock": { "type": "unlimited" },
                    },
                    {
                        "type": "sells",
                        "item": "tusk.item.json",
                        "price": [{ "item": "gold.item.json", "count": 3 }],
                        "stock": { "type": "limited", "count": 3, "restock": 600.0 },
                    },
                ],
            }),
        )
        .mark("merchant")
        .memory("taunted", "never")
        .edit("class", "hero", |hero| {
            hero["tree"] = json!({
                "guard": { "skill": "guard.skill.json", "at": { "x": 0.0, "y": 0.0 } },
                "punch": { "skill": "punch.skill.json", "at": { "x": 80.0, "y": 0.0 } },
            });
            hero["levels"] = json!([{ "xp": 0, "skill_points": 2 }]);
        })
        .row(
            "skill",
            "guard",
            json!({
                "name": "Guard",
                "icon": "icons/hero.png",
                "target": "self",
                "cooldown": 600.0,
                "levels": [{ "cost": 1 }],
                "modules": [{ "type": "script", "script": "mends.script.json" }],
            }),
        )
        .script(
            "mends",
            "skill",
            json!({
                "used": { "type": "on", "event": "used", "next": "mend" },
                "mend": { "type": "heal", "who": { "type": "skill_targets" }, "by": 1.0 },
            }),
        )
        .script(
            "waves",
            "command",
            json!({
                "sent": { "type": "on", "event": "invoked", "next": "wave" },
                "wave": { "type": "heal", "by": 1.0 },
            }),
        )
        .row(
            "command",
            "wave",
            json!({ "script": "waves.script.json", "cooldown": 600.0 }),
        )
        .row(
            "skill",
            "punch",
            json!({
                "name": "Punch",
                "icon": "icons/hero.png",
                "target": "single_enemy",
                "basic_attack": true,
                "levels": [{ "cost": 1 }],
                "modules": [{ "type": "script", "script": "strikes.script.json" }],
            }),
        )
        .script(
            "strikes",
            "skill",
            json!({
                "used": { "type": "on", "event": "used", "next": "hit" },
                "hit": { "type": "damage", "who": { "type": "skill_targets" }, "by": 1.0 },
            }),
        )
        .item("mark", json!({ "kind": { "type": "resource" }, "modules": [{ "type": "stack", "max": 50 }] }))
        .item("beat", json!({ "kind": { "type": "resource" }, "modules": [{ "type": "stack", "max": 50 }] }))
        .row(
            "status_effect",
            "blessed",
            json!({
                "name": "Blessed",
                "icon": "icons/hero.png",
                "category": "buff",
                "modules": [{ "type": "script", "script": "blessed_holds.script.json" }],
            }),
        )
        .script(
            "blessed_holds",
            "status_effect",
            json!({
                "activated": { "type": "on", "event": "activated", "next": "hold" },
                "hold": { "type": "boost", "stat": "max_health", "by": { "type": "status_power" }, "next": "mark" },
                "mark": { "type": "items", "give": [{ "item": "mark.item.json" }] },
                "every": { "type": "on", "event": "interval", "seconds": 2.0, "next": "beat" },
                "beat": { "type": "items", "give": [{ "item": "beat.item.json" }] },
            }),
        )
        .shader("status_pop")
        .load();
    let mut sim = Sim::new(content);
    sim.serve(area)
        .add(status::expire)
        .plan((
            status::aspect::SettleAspects::default(),
            status::aspect::TickAspects::default(),
        ))
        .plan(engine::core::script::run::FlowRunner::default());
    engine::systems::spawn_scripts(&mut sim.world);
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

/// A character with every part of one filled. It names each field, so a part added to a
/// character is a part these tests fill.
fn tess(content: &Content, start: Place) -> CharacterSnapshot {
    CharacterSnapshot {
        name: "Tess".to_owned(),
        class: id(content, "hero"),
        appearance: Appearance::of(id(content, "hero")),
        xp: 60,
        skills: vec![
            SnapshotSkill {
                node: NodeKey("guard".to_owned()),
                rank: Rank::FIRST,
            },
            SnapshotSkill {
                node: NodeKey("punch".to_owned()),
                rank: Rank::FIRST,
            },
        ],
        recipe_masteries: SavedList::default(),
        hotbar: vec![SnapshotSlot {
            slot: HotbarSlot(2),
            entry: HotbarEntry::Skill(id(content, "guard")),
        }]
        .into(),
        basic_attack: Some(id(content, "punch")),
        stats: Stats(BTreeMap::from([(StatKind::Damage, 9.0)])),
        bag: vec![
            BagEntry::of(0, ItemStack::new(id(content, "tusk"), 5)),
            BagEntry::of(3, ItemStack::new(id(content, "gold"), 20)),
        ]
        .into(),
        equipped: vec![id(content, "shield")].into(),
        known_areas: vec![id(content, "village"), id(content, "forest")].into(),
        quests: vec![
            SnapshotQuest {
                progress: vec![0, 1],
                left: Some(Seconds(900.0)),
                dry_kills: 3,
                told_ready: true,
                ..SnapshotQuest::of(id(content, "errand"), QuestState::Active)
            },
            SnapshotQuest {
                days_ago: 2,
                resets_in: Some(Seconds(1200.0)),
                offered_again: true,
                ..SnapshotQuest::of(id(content, "letter"), QuestState::Completed)
            },
        ]
        .into(),
        tracked_quests: None,
        memories: vec![SnapshotMemory {
            memory: id(content, "taunted"),
            value: MemoryValue::Bool(true),
            ago: Seconds(10.0),
        }]
        .into(),
        lines_heard: vec![HeardLine {
            script: "npc/crier".to_owned(),
            node: "news".to_owned(),
        }],
        shops: vec![SnapshotShop {
            shop: id(content, "wares"),
            stock: vec![ShopStock {
                ware: 1,
                left: 2,
                restocks_in: Seconds(400.0),
            }],
            sales: vec![sale(content)],
        }]
        .into(),
        history: vec![SnapshotRecord {
            ago: Seconds(5.0),
            entry: HistoryEntry::of(HistoryTopic::Talk, hello()),
        }],
        statuses: SavedList::default(),
        cooldowns: vec![SnapshotCooldown {
            skill: id(content, "guard"),
            left: Seconds(300.0),
        }]
        .into(),
        command_cooldowns: vec![SnapshotCommandCooldown {
            command: id(content, "wave"),
            left: Seconds(300.0),
        }]
        .into(),
        instance_cooldowns: vec![SnapshotInstanceCooldown {
            area: id(content, "forest"),
            left: Seconds(900.0),
        }]
        .into(),
        rises_at: None,
        start: CharacterStart::At(start),
    }
}

fn hello() -> LineText {
    LineText::plain("Hello")
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
