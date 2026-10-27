use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use engine::systems::actor::skeleton::EquipmentSlot;
use engine::systems::equipment::{self, Equipment, EquipmentPlace, UnequipRequest};
use engine::systems::item::{ItemDef, ItemId};
use engine::systems::stat::{self, StatKind, Stats};
use engine::systems::{script, status};
use serde_json::json;
use testkit::{BODY, Game, Map, Sim};

#[test]
fn a_lowered_maximum_pulls_health_down_to_it() {
    let (mut sim, player) = wardrobe();
    set_vitals(&mut sim, player, 30.0, 30.0);
    sim.give(player, "rock", 1);

    let rock = sim.id::<ItemDef>("rock");
    equipment::equip_from_bag(&mut sim.world, player, rock).expect("the rock is worn");
    sim.tick();

    assert_eq!(stat::max_health(&sim.world, player), 20.0);
    assert_eq!(stat::current_health(&sim.world, player), 20.0);
}

#[test]
fn healing_does_not_bring_the_dead_back() {
    let (mut sim, player) = wardrobe();
    set_vitals(&mut sim, player, 0.0, 30.0);

    stat::heal(&mut sim.world, player, 10.0);
    stat::refill(&mut sim.world, player);

    assert!(stat::is_dead(&sim.world, player));
}

#[test]
fn the_dead_cannot_take_off_what_they_wear() {
    let (mut sim, player) = wardrobe();
    sim.give(player, "rock", 1);
    let rock = sim.id::<ItemDef>("rock");
    equipment::equip_from_bag(&mut sim.world, player, rock).expect("the rock is worn");
    set_vitals(&mut sim, player, 0.0, 30.0);

    let conn = sim.conn(1);
    sim.send(conn, UnequipRequest { slot: head() });
    sim.tick();

    assert_eq!(worn(&sim, player, head()), Some(rock));
    assert_eq!(sim.count(player, "rock"), 0);
}

#[test]
fn wearing_one_of_a_stack_leaves_the_rest_in_the_bag() {
    let (mut sim, player) = wardrobe();
    sim.give(player, "rock", 2);
    let rock = sim.id::<ItemDef>("rock");

    equipment::equip_from_bag(&mut sim.world, player, rock).expect("the rock is worn");

    assert_eq!(worn(&sim, player, head()), Some(rock));
    assert_eq!(sim.count(player, "rock"), 1);
}

#[test]
fn an_item_for_a_slot_the_body_lacks_stays_in_the_bag() {
    let (mut sim, player) = wardrobe();
    sim.give(player, "boots", 1);
    let boots = sim.id::<ItemDef>("boots");

    let refused = equipment::equip_from_bag(&mut sim.world, player, boots).expect_err("refused");

    assert_eq!(refused, "boots can't be worn by this body");
    assert_eq!(sim.count(player, "boots"), 1);
    assert!(!equipment::wears(&sim.world, player, boots));
}

fn wardrobe() -> (Sim, Entity) {
    let content = Game::new()
        .area("room", &Map::open(5, 5), json!([]))
        .hero("room", 2.0, 2.0)
        .slots(BODY, &["head"])
        .item(
            "rock",
            json!({
                "kind": { "type": "equipment", "wear": "worn", "slot": "head" },
                "modules": [{ "type": "script", "script": "rock_lower.script.json" }],
            }),
        )
        .item(
            "boots",
            json!({ "kind": { "type": "equipment", "wear": "worn", "slot": "feet" } }),
        )
        .script(
            "rock_lower",
            "item",
            json!({
                "activated": { "type": "on", "event": "activated", "next": "lower" },
                "lower": { "type": "boost", "stat": "max_health", "by": -10.0 },
            }),
        )
        .load();
    let mut sim = Sim::new(content);
    sim.serve("room")
        .accept::<UnequipRequest>()
        .add((
            (status::expire, status::aspect::settle, status::aspect::tick).chain(),
            equipment::unequip,
        ))
        .add(script::run_tick)
        .add(stat::settle_health);
    script::spawn_stateless(&mut sim.world);
    let player = sim.join(1);
    (sim, player)
}

fn head() -> EquipmentPlace {
    EquipmentPlace::Slot(EquipmentSlot::new("head"))
}

fn set_vitals(sim: &mut Sim, player: Entity, health: f32, max: f32) {
    *sim.world.get_mut::<Stats>(player).expect("stats") = Stats(BTreeMap::from([
        (StatKind::Health, health),
        (StatKind::MaxHealth, max),
    ]));
}

fn worn(sim: &Sim, player: Entity, slot: EquipmentPlace) -> Option<ItemId> {
    sim.world
        .get::<Equipment>(player)
        .expect("equipment")
        .slots
        .get(&slot)
        .copied()
}
