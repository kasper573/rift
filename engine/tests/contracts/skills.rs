use bevy_ecs::prelude::*;
use engine::core::math::{Offset, Pos};
use engine::core::units::Meters;
use engine::systems::area::AreaDef;
use engine::systems::class::{self, NodeKey, SkillPoints};
use engine::systems::combat::{self, AttackRequest, Attitude};
use engine::systems::equipment::{self, Equipment, EquipmentSlot, UnequipRequest};
use engine::systems::item::{self, ItemDef, UseItemRequest};
use engine::systems::movement::{self, position};
use engine::systems::npc::{self, NpcDef, Pack};
use engine::systems::skill::{
    self, AssignBasicAttackRequest, AssignHotbarRequest, BasicAttack, BasicAttackPick, CastAim,
    Hotbar, HotbarEntry, HotbarSlot, LearnSkillsRequest, Learned, Rank, ResetAnswerRequest,
    ResetPrompt, SkillDef, SkillId, UseSkillRequest,
};
use engine::systems::stat::{self, StatKind};
use engine::systems::{actor, reach, script, vfx};
use serde_json::{Value, json};
use testkit::{Game, Map, Sim, path};

#[test]
fn a_fresh_fighter_wears_the_sword_and_its_strike_fills_the_basic_attack_slot() {
    let (sim, player) = yard();
    let strike = sim.id::<SkillDef>("strike");

    assert_eq!(
        sim.world.get::<BasicAttack>(player),
        Some(&BasicAttack(Some(strike)))
    );
    assert_eq!(sim.world.get::<Hotbar>(player), Some(&Hotbar::default()));
    assert_eq!(sim.world.get::<Learned>(player), Some(&Learned::default()));
    assert_eq!(class::points_to_spend(&sim.world, player), SkillPoints(1));
}

#[test]
fn learning_takes_the_whole_plan_or_nothing() {
    let (mut sim, player) = yard();

    request(&mut sim, learn(&[("cleave", 1), ("guard", 1)]));
    assert_eq!(sim.world.get::<Learned>(player), Some(&Learned::default()));
    assert!(
        sim.errors(1)
            .iter()
            .any(|text| text.contains("skill points"))
    );

    request(&mut sim, learn(&[("cleave", 1)]));
    assert_eq!(learned(&sim, player, "cleave"), Rank(1));
    assert_eq!(class::points_to_spend(&sim.world, player), SkillPoints(0));

    class::grant_xp(&mut sim.world, player, 30);
    sim.run(0.5);
    assert_eq!(class::points_to_spend(&sim.world, player), SkillPoints(1));
    request(&mut sim, learn(&[("second_wind", 1)]));
    assert_eq!(learned(&sim, player, "second_wind"), Rank(0));
    assert!(
        sim.errors(1)
            .iter()
            .any(|text| text.contains("Guard rank 1"))
    );

    request(&mut sim, learn(&[("cleave", 2)]));
    assert_eq!(learned(&sim, player, "cleave"), Rank(2));
    assert_eq!(class::points_to_spend(&sim.world, player), SkillPoints(0));
}

#[test]
fn a_slot_takes_only_what_you_have_and_keeps_it_when_it_goes() {
    let (mut sim, player) = yard();
    let potion = HotbarEntry::Item(sim.id::<ItemDef>("potion"));
    let cleave = skill_entry(&sim, "cleave");
    let strike = skill_entry(&sim, "strike");

    request(&mut sim, assign(0, cleave));
    request(&mut sim, assign(1, potion));
    assert_eq!(sim.world.get::<Hotbar>(player), Some(&Hotbar::default()));
    assert_eq!(sim.errors(1).len(), 2);

    sim.give(player, "potion", 1);
    request(&mut sim, assign(0, strike));
    request(&mut sim, assign(1, potion));
    let bar = sim.world.get::<Hotbar>(player).expect("hotbar");
    assert_eq!(bar.entry(HotbarSlot(0)), strike);
    assert_eq!(bar.entry(HotbarSlot(1)), potion);

    request(
        &mut sim,
        UnequipRequest {
            slot: EquipmentSlot::Weapon,
        },
    );
    assert_eq!(
        sim.world
            .get::<Hotbar>(player)
            .map(|bar| bar.entry(HotbarSlot(0))),
        Some(strike)
    );
    assert_eq!(
        sim.world.get::<BasicAttack>(player),
        Some(&BasicAttack(None))
    );

    let sword = sim.slot_of(player, "sword");
    request(&mut sim, UseItemRequest { slot: sword });
    assert_eq!(
        basic_attack(&sim, player),
        Some(sim.id::<SkillDef>("strike"))
    );
}

#[test]
fn the_basic_attack_slot_accepts_only_known_flagged_skills() {
    let (mut sim, player) = yard();
    let strike = sim.id::<SkillDef>("strike");
    let cleave = sim.id::<SkillDef>("cleave");

    request(
        &mut sim,
        AssignBasicAttackRequest {
            pick: BasicAttackPick::Skill(cleave),
        },
    );
    assert_eq!(basic_attack(&sim, player), Some(strike));

    request(&mut sim, learn(&[("cleave", 1)]));
    request(
        &mut sim,
        AssignBasicAttackRequest {
            pick: BasicAttackPick::Skill(cleave),
        },
    );
    assert_eq!(learned(&sim, player, "cleave"), Rank(1));
    assert_eq!(basic_attack(&sim, player), Some(strike));
    assert!(
        sim.errors(1)
            .iter()
            .any(|text| text.contains("basic attack"))
    );

    request(
        &mut sim,
        AssignBasicAttackRequest {
            pick: BasicAttackPick::Empty,
        },
    );
    assert_eq!(basic_attack(&sim, player), None);
}

#[test]
fn a_weapon_put_in_the_basic_attack_slot_is_worn_and_lends_its_attack() {
    let (mut sim, player) = yard();
    let strike = sim.id::<SkillDef>("strike");
    let sword = sim.id::<ItemDef>("sword");
    let weapon = |sim: &Sim| {
        sim.world
            .get::<Equipment>(player)
            .and_then(|equipment| equipment.slots.get(&EquipmentSlot::Weapon).copied())
    };

    request(
        &mut sim,
        UnequipRequest {
            slot: EquipmentSlot::Weapon,
        },
    );
    assert_eq!(basic_attack(&sim, player), None);

    request(
        &mut sim,
        AssignBasicAttackRequest {
            pick: BasicAttackPick::Weapon(sword),
        },
    );
    assert_eq!(basic_attack(&sim, player), Some(strike));
    assert_eq!(weapon(&sim), Some(sword));
    assert!(sim.errors(1).is_empty());

    request(
        &mut sim,
        AssignBasicAttackRequest {
            pick: BasicAttackPick::Empty,
        },
    );
    request(
        &mut sim,
        AssignBasicAttackRequest {
            pick: BasicAttackPick::Weapon(sword),
        },
    );
    assert_eq!(basic_attack(&sim, player), Some(strike));
    assert!(sim.errors(1).is_empty());

    sim.give(player, "potion", 1);
    let potion = sim.id::<ItemDef>("potion");
    request(
        &mut sim,
        AssignBasicAttackRequest {
            pick: BasicAttackPick::Weapon(potion),
        },
    );
    assert_eq!(basic_attack(&sim, player), Some(strike));
    assert!(
        sim.errors(1)
            .iter()
            .any(|text| text.contains("no basic attack"))
    );
}

#[test]
fn slotting_what_the_bar_already_holds_moves_it() {
    let (mut sim, player) = yard();
    let potion = HotbarEntry::Item(sim.id::<ItemDef>("potion"));
    let strike = skill_entry(&sim, "strike");
    sim.give(player, "potion", 2);

    request(&mut sim, assign(0, potion));
    request(&mut sim, assign(3, potion));
    request(&mut sim, assign(1, strike));
    let bar = sim.world.get::<Hotbar>(player).expect("hotbar").clone();
    assert_eq!(bar.entry(HotbarSlot(0)), HotbarEntry::Empty);
    assert_eq!(bar.entry(HotbarSlot(3)), potion);

    request(&mut sim, assign(1, potion));
    request(&mut sim, assign(3, strike));
    let bar = sim.world.get::<Hotbar>(player).expect("hotbar");
    assert_eq!(bar.entry(HotbarSlot(1)), potion);
    assert_eq!(bar.entry(HotbarSlot(3)), strike);
}

#[test]
fn an_empty_basic_attack_slot_says_so_on_a_click() {
    let (mut sim, player) = yard();
    let dummy = dummy(&mut sim, player, Offset::new(1.0, 0.0));
    let full = stat::effective(&sim.world, dummy, StatKind::Health);

    request(
        &mut sim,
        AssignBasicAttackRequest {
            pick: BasicAttackPick::Empty,
        },
    );
    request(&mut sim, AttackRequest { target: dummy });

    assert!(
        sim.errors(1)
            .iter()
            .any(|text| text == "Nothing to attack with")
    );
    assert_eq!(stat::effective(&sim.world, dummy, StatKind::Health), full);
}

#[test]
fn clicking_another_player_does_nothing_hostile() {
    let (mut sim, player) = yard();
    let other = sim.join(2);
    let full = stat::effective(&sim.world, other, StatKind::Health);
    let strike = sim.id::<SkillDef>("strike");

    let conn = sim.conn(1);
    sim.send(conn, AttackRequest { target: other });
    sim.send(
        conn,
        UseSkillRequest {
            skill: strike,
            aim: CastAim::Actor(other),
        },
    );
    sim.run(0.5);

    assert_eq!(reach::attacking(&sim.world, player), None);
    assert_eq!(stat::effective(&sim.world, other, StatKind::Health), full);
    assert!(
        sim.errors(1)
            .iter()
            .any(|text| text.contains("can't target"))
    );
}

#[test]
fn an_area_skill_walks_into_range_lands_at_the_point_and_cools_down() {
    let (mut sim, player) = yard();
    request(&mut sim, learn(&[("cleave", 1)]));
    let dummy = dummy(&mut sim, player, Offset::new(4.0, 0.0));
    let full = stat::effective(&sim.world, dummy, StatKind::Health);
    let at = position(&sim.world, dummy).expect("position");
    let cleave = sim.id::<SkillDef>("cleave");

    cast(&mut sim, cleave, CastAim::Point(at));
    assert!(sim.run_until(10.0, |world| {
        stat::effective(world, dummy, StatKind::Health) < full
    }));
    let struck = stat::effective(&sim.world, dummy, StatKind::Health);

    cast(&mut sim, cleave, CastAim::Point(at));
    sim.run(0.5);
    assert!(
        sim.errors(1)
            .iter()
            .any(|text| text.contains("cooling down"))
    );
    assert_eq!(stat::effective(&sim.world, dummy, StatKind::Health), struck);
}

#[test]
fn a_resource_skill_needs_its_cost_and_eats_it_on_use() {
    let (mut sim, player) = yard();
    request(&mut sim, learn(&[("throw_knife", 1)]));
    let dummy = dummy(&mut sim, player, Offset::new(2.0, 0.0));
    let full = stat::effective(&sim.world, dummy, StatKind::Health);
    let throw = sim.id::<SkillDef>("throw_knife");

    cast(&mut sim, throw, CastAim::Actor(dummy));
    sim.run(0.5);
    assert!(sim.errors(1).iter().any(|text| text.contains("knife")));
    assert_eq!(stat::effective(&sim.world, dummy, StatKind::Health), full);

    sim.give(player, "knife", 2);
    cast(&mut sim, throw, CastAim::Actor(dummy));
    assert!(sim.run_until(10.0, |world| {
        stat::effective(world, dummy, StatKind::Health) < full
    }));
    assert_eq!(sim.count(player, "knife"), 1);
}

#[test]
fn a_single_target_cast_engages_its_target_while_walking_and_lets_go_after() {
    let (mut sim, player) = yard();
    request(&mut sim, learn(&[("throw_knife", 1)]));
    sim.give(player, "knife", 1);
    let dummy = dummy(&mut sim, player, Offset::new(8.0, 0.0));
    let full = stat::effective(&sim.world, dummy, StatKind::Health);
    let throw = sim.id::<SkillDef>("throw_knife");

    cast(&mut sim, throw, CastAim::Actor(dummy));
    sim.tick();
    sim.tick();
    assert_eq!(
        sim.world.get::<reach::Engaged>(player),
        Some(&reach::Engaged(dummy))
    );
    assert!(sim.run_until(10.0, |world| {
        stat::effective(world, dummy, StatKind::Health) < full
    }));
    sim.run(0.5);
    assert_eq!(sim.world.get::<reach::Engaged>(player), None);
}

#[test]
fn a_reset_asks_first_and_gives_every_point_back() {
    let (mut sim, player) = yard();
    request(&mut sim, learn(&[("cleave", 1)]));
    sim.give(player, "draught", 1);
    let slot = sim.slot_of(player, "draught");

    request(&mut sim, UseItemRequest { slot });
    assert_eq!(
        sim.world.get::<ResetPrompt>(player),
        Some(&ResetPrompt {
            ranks: 1,
            skills: 1,
            points: SkillPoints(1),
        })
    );

    request(&mut sim, ResetAnswerRequest { accepted: false });
    assert_eq!(sim.world.get::<ResetPrompt>(player), None);
    assert_eq!(learned(&sim, player, "cleave"), Rank(1));
    assert_eq!(sim.count(player, "draught"), 1);

    request(&mut sim, UseItemRequest { slot });
    request(&mut sim, ResetAnswerRequest { accepted: true });
    assert_eq!(learned(&sim, player, "cleave"), Rank(0));
    assert_eq!(class::points_to_spend(&sim.world, player), SkillPoints(1));
    assert_eq!(sim.count(player, "draught"), 0);
}

/// A fighter in a yard, wearing a sword whose strike is their basic attack, with a tree of
/// skills to learn: an area sweep, a guard, a second wind that needs both, and a knife throw
/// that costs a knife.
fn yard() -> (Sim, Entity) {
    let content = Game::new()
        .area("yard", &Map::open(15, 9), json!([]))
        .hero("yard", 4.0, 4.0)
        .sword()
        .wearing(&["sword"])
        .edit("class", "hero", |hero| {
            hero["levels"] = json!([
                { "xp": 0, "skill_points": 1 },
                { "xp": 30, "skill_points": 1 },
            ]);
            hero["tree"] = json!({
                "cleave": { "skill": "cleave.skill.json", "at": { "x": 0.0, "y": 0.0 } },
                "guard": { "skill": "guard.skill.json", "at": { "x": 80.0, "y": 0.0 } },
                "second_wind": {
                    "skill": "second_wind.skill.json",
                    "requires": [{ "type": "skill", "node": "cleave" }, { "type": "skill", "node": "guard" }],
                    "at": { "x": 40.0, "y": 80.0 },
                },
                "throw_knife": { "skill": "throw_knife.skill.json", "at": { "x": 160.0, "y": 0.0 } },
            });
        })
        .row(
            "skill",
            "cleave",
            skill(
                "Cleave",
                json!({
                    "target": "area_enemy",
                    "area": { "type": "cone", "angle": 90.0, "length": 2.0 },
                    "cooldown": 6.0,
                    "levels": [{ "cost": 1, "stats": {} }, { "cost": 1, "stats": {} }],
                }),
                "cleave_strike",
            ),
        )
        .script("cleave_strike", "skill", strike_nodes())
        .row(
            "skill",
            "throw_knife",
            skill(
                "Throw Knife",
                json!({
                    "target": "single_enemy",
                    "range": 4.0,
                    "cooldown": 2.0,
                    "cost": [{ "item": "knife.item.json" }],
                    "levels": [{ "cost": 1, "stats": {} }],
                }),
                "throw_knife_strike",
            ),
        )
        .script("throw_knife_strike", "skill", strike_nodes())
        .row("skill", "guard", self_skill("Guard", "guard_breathe"))
        .script("guard_breathe", "skill", breathing())
        .row("skill", "second_wind", self_skill("Second Wind", "second_wind_breathe"))
        .script("second_wind_breathe", "skill", breathing())
        .asset("icons/skill.png", testkit::png(1, 1))
        .item(
            "potion",
            json!({
                "kind": { "type": "usable" },
                "modules": [{ "type": "stack", "max": 5 }, { "type": "script", "script": "potion_drink.script.json" }],
            }),
        )
        .script(
            "potion_drink",
            "item",
            json!({
                "used": { "type": "on", "event": "used", "next": "drink" },
                "drink": { "type": "items", "take": [{ "item": "potion.item.json" }] },
            }),
        )
        .item("knife", json!({ "kind": { "type": "resource" }, "modules": [{ "type": "stack", "max": 20 }] }))
        .item(
            "draught",
            json!({
                "kind": { "type": "usable" },
                "modules": [{ "type": "script", "script": "draught_drink.script.json" }],
            }),
        )
        .script(
            "draught_drink",
            "item",
            json!({
                "used": { "type": "on", "event": "used", "next": "ask" },
                "ask": { "type": "reset_skills", "accepted": "drink" },
                "drink": { "type": "items", "take": [{ "item": "draught.item.json" }] },
            }),
        )
        .npc("dummy", 100.0, json!([]))
        .vfx("damage_taken")
        .shader("damage_pop")
        .vfx("level_up")
        .shader("level_pop")
        .vfx("defeated")
        .sfx("ui_refuse")
        .sfx("rising_chime")
        .load();
    let mut sim = Sim::new(content);
    sim.serve("yard")
        .accept::<AttackRequest>()
        .accept::<UseSkillRequest>()
        .accept::<LearnSkillsRequest>()
        .accept::<AssignHotbarRequest>()
        .accept::<AssignBasicAttackRequest>()
        .accept::<ResetAnswerRequest>()
        .accept::<UseItemRequest>()
        .accept::<UnequipRequest>()
        .add((
            vfx::expire,
            combat::request,
            skill::requests,
            item::use_item,
            equipment::unequip,
            skill::reset::expire,
            actor::playing::progress,
            skill::casting::casts,
            reach::mirror_engaged,
        ))
        .add((
            script::query::take_tick_roster,
            script::run_tick,
            movement::advance,
        ))
        .add((
            stat::settle_health,
            combat::defeats,
            skill::hotbar::settle_basic_attack,
            actor::settle,
        ));
    script::spawn_stateless(&mut sim.world);
    let player = sim.join(1);
    sim.run(0.5);
    (sim, player)
}

fn skill(name: &str, fields: Value, script: &str) -> Value {
    let mut row = json!({
        "name": name,
        "icon": "icons/skill.png",
        "modules": [{ "type": "script", "script": path("script", script) }],
    });
    for (field, value) in fields.as_object().expect("a skill's fields") {
        row[field] = value.clone();
    }
    row
}

fn self_skill(name: &str, script: &str) -> Value {
    skill(
        name,
        json!({ "target": "self", "cooldown": 20.0, "levels": [{ "cost": 1, "stats": {} }] }),
        script,
    )
}

fn breathing() -> Value {
    json!({
        "used": { "type": "on", "event": "used", "next": "breathe" },
        "breathe": { "type": "heal", "who": { "type": "skill_targets" }, "by": { "type": "stat", "stat": "damage" } },
    })
}

fn strike_nodes() -> Value {
    json!({
        "used": { "type": "on", "event": "used", "next": "swing" },
        "swing": { "type": "animate", "action": "attack", "wait": "apex", "next": "hit" },
        "hit": { "type": "damage", "who": { "type": "skill_targets" }, "by": { "type": "stat", "stat": "damage" } },
    })
}

/// Sends the first client's request and lets the world answer it.
fn request<M: bevy_ecs::message::Message>(sim: &mut Sim, message: M) {
    let conn = sim.conn(1);
    sim.send(conn, message);
    sim.run(0.5);
}

fn cast(sim: &mut Sim, skill: SkillId, aim: CastAim) {
    let conn = sim.conn(1);
    sim.send(conn, UseSkillRequest { skill, aim });
}

fn learn(picks: &[(&str, u8)]) -> LearnSkillsRequest {
    LearnSkillsRequest {
        ranks: picks
            .iter()
            .map(|(node, rank)| (NodeKey::from(*node), Rank(*rank)))
            .collect(),
    }
}

fn assign(slot: u32, entry: HotbarEntry) -> AssignHotbarRequest {
    AssignHotbarRequest {
        slot: HotbarSlot(slot),
        entry,
    }
}

fn learned(sim: &Sim, player: Entity, node: &str) -> Rank {
    sim.world
        .get::<Learned>(player)
        .map_or(Rank(0), |learned| learned.rank(node))
}

fn basic_attack(sim: &Sim, player: Entity) -> Option<SkillId> {
    sim.world.get::<BasicAttack>(player).and_then(|slot| slot.0)
}

fn skill_entry(sim: &Sim, key: &str) -> HotbarEntry {
    HotbarEntry::Skill(sim.id::<SkillDef>(key))
}

/// A monster turned hostile that stands where it's put and never strikes back.
fn dummy(sim: &mut Sim, player: Entity, offset: Offset<Meters>) -> Entity {
    let at: Pos<Meters> = position(&sim.world, player).expect("the player") + offset;
    let area = sim.id::<AreaDef>("yard");
    let def = sim.id::<NpcDef>("dummy");
    let dummy = npc::spawn(&mut sim.world, def, at, area, Pack(u32::MAX));
    sim.world.entity_mut(dummy).insert(Attitude::Hostile);
    sim.tick();
    dummy
}
