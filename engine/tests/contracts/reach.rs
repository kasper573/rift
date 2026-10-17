use bevy_ecs::prelude::*;
use engine::core::math::Pos;
use engine::core::tiling::Tiles;
use engine::core::time::Seconds;
use engine::systems::area::AreaTag;
use engine::systems::item::{DroppedItem, Inventory, PickupRequest, Reservation};
use engine::systems::movement::{MoveRequest, MoveSeq, Position, position};
use engine::systems::npc::{self, Pack};
use engine::systems::reach::{self, ReachAct, ReachTarget, Tether};

use testkit::{Sim, row, spawn_area};

#[test]
fn a_pickup_walks_into_reach_and_then_collects() {
    let mut sim = Sim::area(spawn_area());
    let player = sim.join(1);
    let start = position(sim.world(), player).expect("position");
    let spot = sim.walkable_near(start, Tiles(3.0), Tiles(5.0));
    let item = drop_bone(&mut sim, spot);

    sim.send(1, PickupRequest { target: item });

    assert!(sim.run_until(10.0, |world| bones(world, player) == 2));
    assert!(sim.world().get_entity(item).is_err());
}

#[test]
fn walking_away_cancels_the_intent() {
    let mut sim = Sim::area(spawn_area());
    let player = sim.join(1);
    let start = position(sim.world(), player).expect("position");
    let spot = sim.walkable_near(start, Tiles(4.0), Tiles(6.0));
    let item = drop_bone(&mut sim, spot);

    sim.send(1, PickupRequest { target: item });
    sim.tick();
    sim.send(
        1,
        MoveRequest {
            pos: start,
            seq: MoveSeq(1),
        },
    );

    assert!(!sim.run_until(10.0, |world| bones(world, player) > 0));
    assert!(sim.world().get_entity(item).is_ok());
}

#[test]
fn an_actor_holds_one_intent_at_a_time() {
    let mut sim = Sim::area(spawn_area());
    let player = sim.join(1);
    let at = position(sim.world(), player).expect("position");
    let orc = npc::spawn(sim.world(), row("Orc"), at, spawn_area(), Pack(u32::MAX));
    let item = drop_bone(&mut sim, at);
    let world = sim.world();

    reach::intend(
        world,
        player,
        ReachTarget::Actor(orc),
        ReachAct::Cast {
            skill: row("Slash"),
            repeat: true,
        },
    );
    reach::intend(world, player, ReachTarget::Actor(item), ReachAct::Pickup);

    assert_eq!(reach::attacking(world, player), None);
    assert_eq!(reach::intent(world, player, ReachAct::Pickup), Some(item));
}

#[test]
fn a_tether_holds_while_its_owner_stays_in_reach_of_a_living_npc() {
    let mut sim = Sim::area(spawn_area());
    let player = sim.join(1);
    let anchor = position(sim.world(), player).expect("position");
    let far = sim.walkable_near(anchor, Tiles(5.0), Tiles(8.0));
    let orc = npc::spawn(sim.world(), row("Orc"), far, spawn_area(), Pack(u32::MAX));
    let world = sim.world();
    let tether = Tether::around(world, player, Tiles(2.0), Some(orc)).expect("tether");

    assert!(tether.holds(world, player));

    world.get_mut::<Position>(player).expect("position").pos = far;
    assert!(!tether.holds(world, player));

    world.get_mut::<Position>(player).expect("position").pos = anchor;
    assert!(tether.holds(world, player));

    world.despawn(orc);
    assert!(!tether.holds(world, player));
}

fn drop_bone(sim: &mut Sim, at: Pos<Tiles>) -> Entity {
    sim.world()
        .spawn((
            Position::at(at),
            AreaTag { area: spawn_area() },
            DroppedItem {
                item: row("Bone"),
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
    world
        .get::<Inventory>(player)
        .map_or(0, |inventory| inventory.count(row("Bone")))
}
