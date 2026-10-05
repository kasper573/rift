use bevy_ecs::prelude::*;
use game::core::tiling::Tiles;
use game::core::time::Seconds;
use game::data;
use game::systems::area::AreaTag;
use game::systems::item::{DroppedItem, Inventory, PickupRequest, Reservation, ReservedBy};
use game::systems::movement::{MoveRequest, Position, position};
use game::systems::npc::{self, Pack};
use game::systems::reach::{self, ReachAct, Tether};

use crate::support::Sim;

fn drop_bone(sim: &mut Sim, at: game::core::math::Pos<Tiles>) -> Entity {
    sim.world()
        .spawn((
            Position { pos: at },
            AreaTag {
                area: data::area::SPAWN_ID,
            },
            DroppedItem {
                item: data::item::Id::Bone,
                count: 2,
            },
            Reservation {
                by: ReservedBy::None,
                at: Seconds(0.0),
            },
        ))
        .id()
}

fn bones(world: &World, player: Entity) -> u32 {
    world
        .get::<Inventory>(player)
        .map_or(0, |inventory| inventory.count(data::item::Id::Bone))
}

#[test]
fn a_pickup_walks_into_reach_and_then_collects() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
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
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    let start = position(sim.world(), player).expect("position");
    let spot = sim.walkable_near(start, Tiles(4.0), Tiles(6.0));
    let item = drop_bone(&mut sim, spot);

    sim.send(1, PickupRequest { target: item });
    sim.tick();
    sim.send(1, MoveRequest { pos: start });

    assert!(!sim.run_until(10.0, |world| bones(world, player) > 0));
    assert!(sim.world().get_entity(item).is_ok());
}

#[test]
fn an_actor_holds_one_intent_at_a_time() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    let at = position(sim.world(), player).expect("position");
    let orc = npc::spawn(
        sim.world(),
        data::npc::Id::Orc,
        at,
        data::area::SPAWN_ID,
        Pack(u32::MAX),
    );
    let item = drop_bone(&mut sim, at);
    let world = sim.world();

    reach::intend(world, player, orc, ReachAct::Attack);
    reach::intend(world, player, item, ReachAct::Pickup);

    assert_eq!(reach::intent(world, player, ReachAct::Attack), None);
    assert_eq!(reach::intent(world, player, ReachAct::Pickup), Some(item));
}

#[test]
fn a_tether_holds_while_its_owner_stays_in_reach_of_a_living_npc() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    let anchor = position(sim.world(), player).expect("position");
    let far = sim.walkable_near(anchor, Tiles(5.0), Tiles(8.0));
    let orc = npc::spawn(
        sim.world(),
        data::npc::Id::Orc,
        far,
        data::area::SPAWN_ID,
        Pack(u32::MAX),
    );
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
