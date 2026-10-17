use bevy_ecs::prelude::*;
use engine::core::math::Offset;
use engine::core::tiling::Tiles;
use engine::core::time::WallClock;
use engine::systems::class::{self, SkillPoints};
use engine::systems::combat::AttackRequest;
use engine::systems::equipment::{EquipmentSlot, UnequipRequest};
use engine::systems::item::{ItemId, UseItemRequest};
use engine::systems::movement::position;
use engine::systems::npc::{self, Pack};
use engine::systems::reach;
use engine::systems::skill::{
    Aim, AssignBasicAttackRequest, AssignHotbarRequest, BasicAttack, Cooldowns, Hotbar,
    HotbarEntry, LearnSkillsRequest, Learned, Rank, ResetAnswerRequest, ResetPrompt, SkillId,
    UseSkillRequest,
};
use engine::systems::stat::{self, StatKind};

use testkit::{Sim, count, errors, give, row, settle, slot_of, spawn_area, target_dummy};

#[test]
fn a_fresh_fighter_wears_the_sword_and_slash_fills_the_basic_attack_slot() {
    let mut sim = Sim::area(spawn_area());
    let player = sim.join(1);
    settle(&mut sim);
    let world = sim.world();

    assert_eq!(
        world.get::<BasicAttack>(player),
        Some(&BasicAttack(Some(row("Slash"))))
    );
    assert_eq!(world.get::<Hotbar>(player), Some(&Hotbar::default()));
    assert_eq!(world.get::<Learned>(player), Some(&Learned::default()));
    assert_eq!(class::points_to_spend(world, player), SkillPoints(1));
}

#[test]
fn learning_takes_the_whole_plan_or_nothing() {
    let mut sim = Sim::area(spawn_area());
    let player = sim.join(1);
    settle(&mut sim);

    sim.send(1, learn(&[("cleave", 1), ("guard", 1)]));
    settle(&mut sim);
    assert_eq!(
        sim.world().get::<Learned>(player),
        Some(&Learned::default())
    );
    assert!(
        errors(&sim, 1)
            .iter()
            .any(|text| text.contains("skill points"))
    );

    sim.send(1, learn(&[("cleave", 1)]));
    settle(&mut sim);
    assert_eq!(learned(&mut sim, player, "cleave"), Rank(1));
    assert_eq!(class::points_to_spend(sim.world(), player), SkillPoints(0));

    class::grant_xp(sim.world(), player, 30);
    settle(&mut sim);
    assert_eq!(class::points_to_spend(sim.world(), player), SkillPoints(1));
    sim.send(1, learn(&[("second_wind", 1)]));
    settle(&mut sim);
    assert_eq!(learned(&mut sim, player, "second_wind"), Rank(0));
    assert!(
        errors(&sim, 1)
            .iter()
            .any(|text| text.contains("Guard rank 1"))
    );

    sim.send(1, learn(&[("cleave", 2)]));
    settle(&mut sim);
    assert_eq!(learned(&mut sim, player, "cleave"), Rank(2));
    assert_eq!(class::points_to_spend(sim.world(), player), SkillPoints(0));
}

#[test]
fn a_slot_takes_only_what_you_have_and_keeps_it_when_it_goes() {
    let mut sim = Sim::area(spawn_area());
    let player = sim.join(1);
    settle(&mut sim);
    let potion: ItemId = row("HealthPotion");

    sim.send(
        1,
        AssignHotbarRequest {
            slot: 0,
            entry: skill_entry("Cleave"),
        },
    );
    sim.send(
        1,
        AssignHotbarRequest {
            slot: 1,
            entry: HotbarEntry::Item(potion),
        },
    );
    settle(&mut sim);
    assert_eq!(sim.world().get::<Hotbar>(player), Some(&Hotbar::default()));
    assert_eq!(errors(&sim, 1).len(), 2);

    give(&mut sim, player, potion, 1);
    sim.send(
        1,
        AssignHotbarRequest {
            slot: 0,
            entry: skill_entry("Slash"),
        },
    );
    sim.send(
        1,
        AssignHotbarRequest {
            slot: 1,
            entry: HotbarEntry::Item(potion),
        },
    );
    settle(&mut sim);
    let bar = sim.world().get::<Hotbar>(player).expect("hotbar");
    assert_eq!(bar.entry(0), skill_entry("Slash"));
    assert_eq!(bar.entry(1), HotbarEntry::Item(potion));

    sim.send(
        1,
        UnequipRequest {
            slot: EquipmentSlot::Weapon,
        },
    );
    settle(&mut sim);
    let world = sim.world();
    assert_eq!(
        world.get::<Hotbar>(player).map(|bar| bar.entry(0)),
        Some(skill_entry("Slash"))
    );
    assert_eq!(world.get::<BasicAttack>(player), Some(&BasicAttack(None)));

    let sword = slot_of(&mut sim, player, row("IronSword"));
    sim.send(1, UseItemRequest { slot: sword });
    settle(&mut sim);
    assert_eq!(
        sim.world().get::<BasicAttack>(player),
        Some(&BasicAttack(Some(row("Slash"))))
    );
}

#[test]
fn the_basic_attack_slot_accepts_only_known_flagged_skills() {
    let mut sim = Sim::area(spawn_area());
    let player = sim.join(1);
    settle(&mut sim);

    sim.send(
        1,
        AssignBasicAttackRequest {
            skill: Some(row("Cleave")),
        },
    );
    settle(&mut sim);
    assert_eq!(basic_attack(&mut sim, player), Some(row("Slash")));

    sim.send(1, learn(&[("cleave", 1)]));
    sim.send(
        1,
        AssignBasicAttackRequest {
            skill: Some(row("Cleave")),
        },
    );
    settle(&mut sim);
    assert_eq!(basic_attack(&mut sim, player), Some(row("Slash")));
    assert!(
        errors(&sim, 1)
            .iter()
            .any(|text| text.contains("basic attack"))
    );

    sim.send(1, AssignBasicAttackRequest { skill: None });
    settle(&mut sim);
    assert_eq!(basic_attack(&mut sim, player), None);
}

#[test]
fn an_empty_basic_attack_slot_says_so_on_a_click() {
    let mut sim = Sim::area(spawn_area());
    let player = sim.join(1);
    settle(&mut sim);
    let orc = orc_near(&mut sim, player, 1.0, 1.0);
    let full = stat::effective(sim.world(), orc, StatKind::Health);

    sim.send(1, AssignBasicAttackRequest { skill: None });
    settle(&mut sim);
    sim.send(1, AttackRequest { target: orc });
    settle(&mut sim);

    assert!(
        errors(&sim, 1)
            .iter()
            .any(|text| text == "Nothing to attack with"),
    );
    assert_eq!(stat::effective(sim.world(), orc, StatKind::Health), full);
}

#[test]
fn clicking_another_player_does_nothing_hostile() {
    let mut sim = Sim::area(spawn_area());
    let player = sim.join(1);
    let other = sim.join(2);
    settle(&mut sim);
    let full = stat::effective(sim.world(), other, StatKind::Health);

    sim.send(1, AttackRequest { target: other });
    sim.send(
        1,
        UseSkillRequest {
            skill: row("Slash"),
            aim: Aim::Actor(other),
        },
    );
    settle(&mut sim);

    assert_eq!(reach::attacking(sim.world(), player), None);
    assert_eq!(stat::effective(sim.world(), other, StatKind::Health), full);
    assert!(
        errors(&sim, 1)
            .iter()
            .any(|text| text.contains("can't target"))
    );
}

#[test]
fn an_area_skill_walks_into_range_lands_at_the_point_and_cools_down() {
    let mut sim = Sim::area(spawn_area());
    let player = sim.join(1);
    settle(&mut sim);
    sim.send(1, learn(&[("cleave", 1)]));
    settle(&mut sim);
    let start = position(sim.world(), player).expect("position");
    let spot = sim.walkable_toward(start, Offset::new(4.0, 0.0));
    let orc = target_dummy(&mut sim, row("Orc"), spot);
    let full = stat::effective(sim.world(), orc, StatKind::Health);
    let at = position(sim.world(), orc).expect("position");

    sim.send(
        1,
        UseSkillRequest {
            skill: row("Cleave"),
            aim: Aim::Point(at),
        },
    );
    assert!(
        sim.run_until(10.0, |world| stat::effective(world, orc, StatKind::Health)
            < full)
    );
    let world = sim.world();
    let now = world.resource::<WallClock>().now;
    let cleave: SkillId = row("Cleave");
    assert!(
        world
            .get::<Cooldowns>(player)
            .is_some_and(|cooldowns| cooldowns.cooling(cleave, now))
    );
    assert_eq!(reach::current(world, player), None);

    sim.send(
        1,
        UseSkillRequest {
            skill: cleave,
            aim: Aim::Point(at),
        },
    );
    settle(&mut sim);
    assert!(
        errors(&sim, 1)
            .iter()
            .any(|text| text.contains("cooling down"))
    );
}

#[test]
fn a_resource_skill_needs_its_cost_and_eats_it_on_use() {
    let mut sim = Sim::area(spawn_area());
    let player = sim.join(1);
    settle(&mut sim);
    sim.send(1, learn(&[("throw_knife", 1)]));
    settle(&mut sim);
    let orc = orc_near(&mut sim, player, 1.0, 2.0);
    let knife: ItemId = row("ThrowingKnife");
    let full = stat::effective(sim.world(), orc, StatKind::Health);

    sim.send(
        1,
        UseSkillRequest {
            skill: row("ThrowKnife"),
            aim: Aim::Actor(orc),
        },
    );
    settle(&mut sim);
    assert!(
        errors(&sim, 1)
            .iter()
            .any(|text| text.contains("Throwing Knife"))
    );
    assert_eq!(stat::effective(sim.world(), orc, StatKind::Health), full);

    give(&mut sim, player, knife, 2);
    sim.send(
        1,
        UseSkillRequest {
            skill: row("ThrowKnife"),
            aim: Aim::Actor(orc),
        },
    );
    assert!(
        sim.run_until(10.0, |world| stat::effective(world, orc, StatKind::Health)
            < full)
    );
    assert_eq!(count(&mut sim, player, knife), 1);
}

#[test]
fn a_single_target_cast_engages_its_target_while_walking_and_lets_go_after() {
    let mut sim = Sim::area(spawn_area());
    let player = sim.join(1);
    settle(&mut sim);
    sim.send(1, learn(&[("throw_knife", 1)]));
    settle(&mut sim);
    give(&mut sim, player, row("ThrowingKnife"), 1);
    let start = position(sim.world(), player).expect("position");
    let spot = sim.walkable_toward(start, Offset::new(8.0, 0.0));
    let orc = target_dummy(&mut sim, row("Orc"), spot);
    let full = stat::effective(sim.world(), orc, StatKind::Health);

    sim.send(
        1,
        UseSkillRequest {
            skill: row("ThrowKnife"),
            aim: Aim::Actor(orc),
        },
    );
    sim.tick();
    sim.tick();
    assert_eq!(
        sim.world().get::<reach::Engaged>(player),
        Some(&reach::Engaged(orc))
    );
    assert!(
        sim.run_until(10.0, |world| stat::effective(world, orc, StatKind::Health)
            < full)
    );
    settle(&mut sim);
    assert_eq!(sim.world().get::<reach::Engaged>(player), None);
}

#[test]
fn a_reset_asks_first_and_gives_every_point_back() {
    let mut sim = Sim::area(spawn_area());
    let player = sim.join(1);
    settle(&mut sim);
    sim.send(1, learn(&[("cleave", 1)]));
    settle(&mut sim);
    let draught: ItemId = row("ShamansDraught");
    give(&mut sim, player, draught, 1);
    let slot = slot_of(&mut sim, player, draught);

    sim.send(1, UseItemRequest { slot });
    settle(&mut sim);
    assert_eq!(
        sim.world().get::<ResetPrompt>(player),
        Some(&ResetPrompt {
            ranks: 1,
            skills: 1,
            points: SkillPoints(1),
        })
    );

    sim.send(1, ResetAnswerRequest { accepted: false });
    settle(&mut sim);
    assert_eq!(sim.world().get::<ResetPrompt>(player), None);
    assert_eq!(learned(&mut sim, player, "cleave"), Rank(1));
    assert_eq!(count(&mut sim, player, draught), 1);

    sim.send(1, UseItemRequest { slot });
    settle(&mut sim);
    sim.send(1, ResetAnswerRequest { accepted: true });
    settle(&mut sim);
    assert_eq!(learned(&mut sim, player, "cleave"), Rank(0));
    assert_eq!(class::points_to_spend(sim.world(), player), SkillPoints(1));
    assert_eq!(count(&mut sim, player, draught), 0);
}

fn learn(picks: &[(&str, u8)]) -> LearnSkillsRequest {
    LearnSkillsRequest {
        ranks: picks
            .iter()
            .map(|(node, rank)| ((*node).to_owned(), Rank(*rank)))
            .collect(),
    }
}

fn learned(sim: &mut Sim, player: Entity, node: &str) -> Rank {
    sim.world()
        .get::<Learned>(player)
        .map_or(Rank(0), |learned| learned.rank(node))
}

#[test]
fn a_filled_hotbar_and_its_requests_survive_the_wire() {
    let bar = Hotbar(vec![
        skill_entry("Cleave"),
        HotbarEntry::Empty,
        HotbarEntry::Item(row("HealthPotion")),
    ]);
    let bytes = postcard::to_allocvec(&bar).expect("serializes");
    assert_eq!(postcard::from_bytes::<Hotbar>(&bytes).expect("reads"), bar);
    let request = AssignHotbarRequest {
        slot: 2,
        entry: HotbarEntry::Item(row("HealthPotion")),
    };
    let bytes = postcard::to_allocvec(&request).expect("serializes");
    let received: AssignHotbarRequest = postcard::from_bytes(&bytes).expect("reads");
    assert_eq!(received, request);
}

fn basic_attack(sim: &mut Sim, player: Entity) -> Option<SkillId> {
    sim.world()
        .get::<BasicAttack>(player)
        .and_then(|slot| slot.0)
}

fn skill_entry(name: &str) -> HotbarEntry {
    HotbarEntry::Skill(row(name))
}

fn orc_near(sim: &mut Sim, player: Entity, min: f32, max: f32) -> Entity {
    let start = position(sim.world(), player).expect("position");
    let spot = sim.walkable_near(start, Tiles(min), Tiles(max));
    let orc = npc::spawn(sim.world(), row("Orc"), spot, spawn_area(), Pack(u32::MAX));
    sim.tick();
    orc
}
