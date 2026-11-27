use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use engine::systems::actor::skeleton::EquipmentSlot;
use engine::systems::equipment::{self, EquipRequest, Equipment, EquipmentPlace, UnequipRequest};
use engine::systems::item::{self, Inventory, ItemDef, ItemId};
use engine::systems::stat::{self, StatKind, Stats, Vital};
use engine::systems::status;
use serde_json::json;
use testkit::{BODY, GEAR, Game, Map, Sim, path};

#[test]
fn a_lowered_maximum_pulls_health_and_mana_down_to_it() {
    let (mut sim, player) = wardrobe();
    set_vitals(&mut sim, player, 30.0, 30.0);
    sim.give(player, "rock", 1);

    let rock = sim.id::<ItemDef>("rock");
    equipment::equip_from_bag(&mut sim.world, player, rock).expect("the rock is worn");
    sim.run(0.1);

    for vital in Vital::BOTH {
        assert_eq!(stat::most(&sim.world, player, vital), 20.0);
        assert_eq!(stat::current(&sim.world, player, vital), 20.0);
    }
}

#[test]
fn healing_does_not_bring_the_dead_back() {
    let (mut sim, player) = wardrobe();
    set_vitals(&mut sim, player, 0.0, 30.0);

    stat::heal(&mut sim.world, player, 10.0);
    stat::refill(&mut sim.world, player, Vital::Health);

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
    sim.send(
        conn,
        UnequipRequest {
            slot: head(),
            into: None,
        },
    );
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
fn an_item_the_body_cannot_wear_stays_in_the_bag_unused() {
    for (name, refusal) in [
        ("boots", "boots can't be worn by this body"),
        ("potion", "potion can't be worn"),
    ] {
        let (mut sim, player) = wardrobe();
        sim.give(player, name, 1);
        let item = sim.id::<ItemDef>(name);

        let refused = equipment::equip_from_bag(&mut sim.world, player, item).expect_err("refused");
        let slot = item::bag_slot(&sim.world, player, item).expect("in the bag");
        let conn = sim.conn(1);
        sim.send(
            conn,
            EquipRequest {
                slot: slot as u32,
                hand: None,
            },
        );
        sim.tick();

        assert_eq!(refused.plain_words(sim.content()), refusal);
        assert_eq!(sim.count(player, name), 1, "{name} stays in the bag");
        assert!(!equipment::wears(&sim.world, player, item));
    }
}

#[test]
fn gear_taken_off_onto_a_slot_of_the_bag_lands_there() {
    let (mut sim, player) = wardrobe();
    for name in ["rock", "hat", "potion"] {
        sim.give(player, name, 1);
    }
    let [rock, hat, potion] = ["rock", "hat", "potion"].map(|name| sim.id::<ItemDef>(name));
    let conn = sim.conn(1);
    let take_off = |sim: &mut Sim, into| {
        sim.send(conn, UnequipRequest { slot: head(), into });
        sim.tick();
    };
    let laid = |sim: &Sim| -> Vec<(usize, ItemId)> {
        sim.world
            .get::<Inventory>(player)
            .expect("a bag")
            .stacks()
            .map(|(slot, stack)| (slot, stack.item))
            .collect()
    };

    equipment::equip_from_bag(&mut sim.world, player, rock).expect("the rock is worn");
    take_off(&mut sim, Some(5));
    assert_eq!(
        laid(&sim),
        [(1, hat), (2, potion), (5, rock)],
        "an empty slot takes the gear"
    );

    equipment::equip_from_bag(&mut sim.world, player, rock).expect("the rock is worn again");
    take_off(&mut sim, Some(1));
    assert_eq!(worn(&sim, player, head()), Some(hat));
    assert_eq!(
        laid(&sim),
        [(1, rock), (2, potion)],
        "gear dropped on something worn in its place swaps with it"
    );

    take_off(&mut sim, Some(2));
    assert_eq!(worn(&sim, player, head()), None);
    assert_eq!(
        laid(&sim),
        [(0, hat), (1, rock), (2, potion)],
        "gear dropped on anything else goes where new items go"
    );
}

fn wardrobe() -> (Sim, Entity) {
    let content = Game::new()
        .area("room", &Map::open(5, 5), json!([]))
        .hero("room", 2.0, 2.0)
        .slots(BODY, &["head"])
        .sfx("ui_refuse")
        .equipment(
            "rock",
            json!({
                "kind": { "type": "equipment", "category": path("equipment_category", GEAR), "wear": "worn", "slot": "head" },
                "modules": [{ "type": "script", "script": "rock_lower.script.json" }],
            }),
        )
        .equipment(
            "hat",
            json!({ "kind": { "type": "equipment", "category": path("equipment_category", GEAR), "wear": "worn", "slot": "head" } }),
        )
        .equipment(
            "boots",
            json!({ "kind": { "type": "equipment", "category": path("equipment_category", GEAR), "wear": "worn", "slot": "feet" } }),
        )
        .item(
            "potion",
            json!({
                "kind": { "type": "usable" },
                "modules": [{ "type": "script", "script": "potion_drunk.script.json" }],
            }),
        )
        .script(
            "potion_drunk",
            "item",
            json!({ "used": { "type": "on", "event": "used", "next": "heal" }, "heal": { "type": "heal", "by": "full" } }),
        )
        .script(
            "rock_lower",
            "item",
            json!({
                "activated": { "type": "on", "event": "activated", "next": "lower" },
                "lower": { "type": "boost", "stat": "max_health", "by": -10.0, "next": "drain" },
                "drain": { "type": "boost", "stat": "max_mana", "by": -10.0 },
            }),
        )
        .load();
    let mut sim = Sim::new(content);
    sim.serve("room")
        .accept::<UnequipRequest>()
        .accept::<EquipRequest>()
        .add(status::expire)
        .add((equipment::unequip, equipment::equip_requested))
        .plan((
            status::aspect::SettleAspects::default(),
            status::aspect::TickAspects::default(),
        ))
        .plan(engine::core::script::run::FlowRunner::default())
        .plan(stat::SettleVitals::default());
    engine::systems::spawn_scripts(&mut sim.world);
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
        (StatKind::Mana, health),
        (StatKind::MaxMana, max),
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
