use bevy_ecs::prelude::*;
use engine::core::time::Seconds;
use engine::systems::actor::skeleton::EquipmentSlot;
use engine::systems::equipment::{self, Equipment, EquipmentPlace, UnequipRequest};
use engine::systems::item::{self, DropItemRequest, ItemDef, UseItemRequest};
use engine::systems::player::{ArrivalHold, CommandLock};
use engine::systems::{script, stat};
use serde_json::json;
use testkit::{BODY, Game, Map, Sim};

#[test]
fn a_command_lock_or_an_arrival_hold_refuses_use_drop_and_unequip_until_it_lifts() {
    refused_while_held_and_done_once_lifted(CommandLock);
    refused_while_held_and_done_once_lifted(ArrivalHold {
        until: Seconds(1000.0),
    });
}

fn camp() -> (Sim, Entity) {
    let content = Game::new()
        .area("camp", &Map::open(5, 5), json!([]))
        .hero("camp", 2.0, 2.0)
        .slots(BODY, &["head"])
        .item(
            "potion",
            json!({
                "kind": { "type": "usable" },
                "modules": [{ "type": "script", "script": "potion_drink.script.json" }],
            }),
        )
        .script(
            "potion_drink",
            "item",
            json!({
                "used": { "type": "on", "event": "used", "next": "drink" },
                "drink": { "type": "items", "take": [{ "item": "potion.item.json" }], "done": "mend" },
                "mend": { "type": "heal", "by": 25.0 },
            }),
        )
        .item("tusk", json!({ "kind": { "type": "resource" } }))
        .item(
            "helmet",
            json!({ "kind": { "type": "equipment", "wear": "worn", "slot": "head" } }),
        )
        .vfx("damage_taken")
        .shader("damage_pop")
        .vfx("healed")
        .shader("heal_pop")
        .load();
    let mut sim = Sim::new(content);
    sim.serve("camp")
        .accept::<UseItemRequest>()
        .accept::<DropItemRequest>()
        .accept::<UnequipRequest>()
        .add((item::use_item, item::drop_item, equipment::unequip))
        .add(script::run_tick)
        .add(stat::settle_health);
    script::spawn_stateless(&mut sim.world);
    let player = sim.join(1);
    (sim, player)
}

fn prepare(sim: &mut Sim, player: Entity) {
    sim.give(player, "potion", 1);
    sim.give(player, "tusk", 1);
    sim.give(player, "helmet", 1);
    let helmet = sim.id::<ItemDef>("helmet");
    equipment::equip_from_bag(&mut sim.world, player, helmet).expect("the helmet is worn");
    stat::apply_damage(&mut sim.world, player, 10.0);
}

fn attempt_each(sim: &mut Sim) {
    let player = sim.player(1);
    let conn = sim.conn(1);
    let potion = sim.slot_of(player, "potion");
    sim.send(conn, UseItemRequest { slot: potion });
    sim.tick();

    let tusk = sim.slot_of(player, "tusk");
    sim.send(conn, DropItemRequest { slot: tusk });
    sim.tick();

    sim.send(
        conn,
        UnequipRequest {
            slot: EquipmentPlace::Slot(EquipmentSlot::new("head")),
        },
    );
    sim.tick();
}

#[derive(Debug, PartialEq)]
struct Belongings {
    potions: u32,
    tusks: u32,
    helmet_worn: bool,
}

const PREPARED: Belongings = Belongings {
    potions: 1,
    tusks: 1,
    helmet_worn: true,
};

const EACH_DONE: Belongings = Belongings {
    potions: 0,
    tusks: 0,
    helmet_worn: false,
};

fn belongings(sim: &Sim, player: Entity) -> Belongings {
    Belongings {
        potions: sim.count(player, "potion"),
        tusks: sim.count(player, "tusk"),
        helmet_worn: sim.world.get::<Equipment>(player).is_some_and(|equipment| {
            equipment
                .slots
                .contains_key(&EquipmentPlace::Slot(EquipmentSlot::new("head")))
        }),
    }
}

fn refused_while_held_and_done_once_lifted<Hold: Component>(hold: Hold) {
    let (mut sim, player) = camp();
    prepare(&mut sim, player);

    sim.world.entity_mut(player).insert(hold);
    attempt_each(&mut sim);
    assert_eq!(belongings(&sim, player), PREPARED);

    sim.world.entity_mut(player).remove::<Hold>();
    attempt_each(&mut sim);
    assert_eq!(belongings(&sim, player), EACH_DONE);
}
