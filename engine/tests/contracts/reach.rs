use bevy_ecs::prelude::*;
use engine::core::math::{Offset, Pos};
use engine::core::time::Seconds;
use engine::core::units::Meters;
use engine::systems::area::{AreaDef, AreaTag};
use engine::systems::combat::{self, AttackRequest, Attitude};
use engine::systems::item::{self, DroppedItem, Inventory, ItemDef, PickupRequest, Reservation};
use engine::systems::movement::{self, MoveRequest, MoveSeq, Position, position};
use engine::systems::npc::{self, NpcDef};
use engine::systems::reach;
use engine::systems::stat::{self, StatKind};
use engine::systems::{actor, skill};
use serde_json::json;
use testkit::{Declared, Game, Map, Sim};

#[test]
fn a_pickup_walks_into_reach_and_then_collects() {
    let mut sim = yard();
    let player = sim.join(1);
    sim.equip(player, "sword");
    let start = position(&sim.world, player).expect("position");
    let bone = drop_bone(&mut sim, start + Offset::new(4.0, 0.0));

    let conn = sim.conn(1);
    sim.send(conn, PickupRequest { target: bone });

    assert!(sim.run_until(10.0, |world| bones(world, player) == 2));
    assert!(sim.world.get_entity(bone).is_err());
}

#[test]
fn walking_away_cancels_the_intent() {
    let mut sim = yard();
    let player = sim.join(1);
    sim.equip(player, "sword");
    let start = position(&sim.world, player).expect("position");
    let bone = drop_bone(&mut sim, start + Offset::new(5.0, 0.0));

    let conn = sim.conn(1);
    sim.send(conn, PickupRequest { target: bone });
    sim.tick();
    sim.send(
        conn,
        MoveRequest {
            pos: start,
            seq: MoveSeq(1),
        },
    );

    assert!(!sim.run_until(10.0, |world| bones(world, player) > 0));
    assert!(sim.world.get_entity(bone).is_ok());
}

#[test]
fn a_new_order_replaces_the_one_in_progress() {
    let mut sim = yard();
    let player = sim.join(1);
    sim.equip(player, "sword");
    let at = position(&sim.world, player).expect("position");
    let brute = brute(&mut sim, at + Offset::new(6.0, 0.0));
    let full = stat::effective(&sim.world, brute, StatKind::Health);
    let bone = drop_bone(&mut sim, at + Offset::new(-1.0, 0.0));

    let conn = sim.conn(1);
    sim.send(conn, AttackRequest { target: brute });
    sim.run(0.3);
    assert_eq!(reach::attacking(&sim.world, player), Some(brute));
    sim.send(conn, PickupRequest { target: bone });

    assert!(sim.run_until(10.0, |world| bones(world, player) == 2));
    sim.run(3.0);
    assert_eq!(reach::attacking(&sim.world, player), None);
    assert_eq!(stat::effective(&sim.world, brute, StatKind::Health), full);
}

fn yard() -> Sim {
    let content = Game::new()
        .area("yard", &Map::open(13, 5), json!([]))
        .hero("yard", 2.0, 2.0)
        .sword()
        .npc("brute", 25.0, json!([]))
        .item(
            "bone",
            json!({ "kind": { "type": "resource" }, "modules": [{ "type": "stack", "max": 10 }] }),
        )
        .load();
    let mut sim = Sim::new(content);
    sim.serve("yard")
        .accept::<MoveRequest>()
        .accept::<PickupRequest>()
        .accept::<AttackRequest>()
        .add((
            movement::move_request,
            item::pickup_request,
            combat::request,
            actor::playing::progress,
            skill::casting::casts,
        ))
        .add((movement::advance, item::pickups))
        .add((
            stat::settle_health,
            combat::defeats,
            skill::hotbar::settle_basic_attack,
            actor::settle,
        ));
    sim
}

fn brute(sim: &mut Sim, at: Pos<Meters>) -> Entity {
    let area = sim.id::<AreaDef>("yard");
    let brute = sim.id::<NpcDef>("brute");
    let brute = npc::spawn(&mut sim.world, brute, at, area, None);
    sim.world.entity_mut(brute).insert(Attitude::Hostile);
    brute
}

fn drop_bone(sim: &mut Sim, at: Pos<Meters>) -> Entity {
    let area = sim.id::<AreaDef>("yard");
    let bone = sim.id::<ItemDef>("bone");
    sim.world
        .spawn((
            Position::at(at),
            AreaTag { area },
            DroppedItem {
                item: bone,
                count: 2,
            },
            Reservation {
                by: None,
                at: Seconds(0.0),
            },
        ))
        .id()
}

fn bones(world: &World, player: Entity) -> u32 {
    let content = world.resource::<engine::core::content::Content>();
    let bone = content.declared::<ItemDef>("bone").expect("bones");
    world
        .get::<Inventory>(player)
        .map_or(0, |inventory| inventory.count(bone))
}
