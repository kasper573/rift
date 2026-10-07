use bevy_ecs::prelude::*;
use game::core::math::{Pos, Rect};
use game::core::tiling::{TilePos, Tiles};
use game::data;
use game::data::memory::Id as MemoryId;
use game::systems::area::transition::Crossing;
use game::systems::dialogue::Conversation;
use game::systems::memory::Memory;
use game::systems::movement::{MoveRequest, MoveToPortal, position};
use game::systems::notification::Notification;
use game::systems::player::commands_locked;

use crate::support::{
    Sim, conversation, count, errors, give, heard_the_news, labels, leave, locked, pick, row,
    spoken, talk, townsperson,
};

fn forest_road(sim: &Sim) -> (u32, Rect<Tiles>) {
    let (index, portal) = sim
        .map()
        .portals
        .iter()
        .enumerate()
        .find(|(_, portal)| portal.name == "forest-road")
        .expect("the forest road on the map");
    (index as u32, portal.rect)
}

fn head_for_the_forest_road(sim: &mut Sim, client: u32) {
    let (portal, road) = forest_road(sim);
    sim.send(
        client,
        MoveToPortal {
            pos: road.center(),
            portal,
        },
    );
}

fn remember(sim: &mut Sim, player: Entity, key: MemoryId) {
    let clock = *sim.world().resource::<game::core::time::WallClock>();
    sim.world()
        .get_mut::<Memory>(player)
        .expect("memory")
        .remember(key, clock);
}

fn halted(sim: &mut Sim, player: Entity) -> bool {
    sim.run_until(15.0, |world| world.get::<Conversation>(player).is_some())
        && conversation(sim, player).is_some_and(|now| now.node == row("IlsaHalt"))
}

fn crossed(sim: &mut Sim, player: Entity, within: f32) -> bool {
    sim.run_until(within, |world| world.get::<Crossing>(player).is_some())
}

#[test]
fn without_a_pass_the_forest_road_is_ground_and_ilsa_halts_you_on_it() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    let ilsa = townsperson(&mut sim, row("Ilsa"));
    let (_, road) = forest_road(&sim);

    head_for_the_forest_road(&mut sim, 1);
    assert!(halted(&mut sim, player), "Ilsa never stopped you");
    let halt = conversation(&mut sim, player).expect("halted");
    assert_eq!(halt.with, Some(ilsa));
    assert!(road.contains(position(sim.world(), player).expect("position")));
    assert!(commands_locked(sim.world(), player));
    assert!(!labels(&halt).contains(&"I have a Road Pass.".to_owned()));
    assert!(locked(&halt, "I'm ready for the forest road."));
    pick(&mut sim, 1, player, "I'm ready for the forest road.");
    assert_eq!(errors(&sim, 1), vec!["Needs Level 3".to_owned()]);

    leave(&mut sim, 1, player);
    head_for_the_forest_road(&mut sim, 1);
    assert!(
        !crossed(&mut sim, player, 3.0),
        "the warp took you without a pass"
    );
    assert!(conversation(&mut sim, player).is_none());
}

#[test]
fn ilsa_halts_you_each_time_you_step_back_onto_the_road() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    let (_, road) = forest_road(&sim);
    head_for_the_forest_road(&mut sim, 1);
    assert!(halted(&mut sim, player));
    leave(&mut sim, 1, player);

    let off = Pos::new(road.center().x - 2.0, road.center().y);
    sim.send(1, MoveRequest { pos: off });
    assert!(sim.run_until(15.0, |world| {
        position(world, player).is_some_and(|at| at.distance(off) < Tiles(0.5))
    }));
    head_for_the_forest_road(&mut sim, 1);
    assert!(halted(&mut sim, player), "Ilsa let you try twice");
}

#[test]
fn a_road_pass_opens_the_forest_road_without_a_word_from_ilsa() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    give(&mut sim, player, row("RoadPass"), 1);

    head_for_the_forest_road(&mut sim, 1);
    assert!(crossed(&mut sim, player, 15.0), "the warp never took you");
    let crossing = *sim.world().get::<Crossing>(player).expect("crossing");
    assert_eq!(crossing.dest_area, row("Forest"));
    assert!(conversation(&mut sim, player).is_none());
}

#[test]
fn walking_past_the_forest_road_is_no_business_of_ilsas() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    let (_, road) = forest_road(&sim);
    let past = Pos::new(road.center().x - 2.0, road.center().y - 1.0);

    sim.send(1, MoveRequest { pos: past });
    assert!(sim.run_until(15.0, |world| {
        position(world, player).is_some_and(|at| at.distance(past) < Tiles(0.5))
    }));
    assert!(conversation(&mut sim, player).is_none());
}

#[test]
fn ilsa_trades_a_road_pass_for_a_fish_and_bram_only_looks_at_it() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    heard_the_news(&mut sim, player);
    give(&mut sim, player, row("FishSteak"), 1);

    let greeting = talk(&mut sim, 1, player, row("Ilsa"));
    assert!(!labels(&greeting).contains(&"Ugra's clan walks with me.".to_owned()));
    let traded = pick(&mut sim, 1, player, "Would a fish change your mind?").expect("talking");
    assert_eq!(traded.node, row("IlsaFish"));
    assert_eq!(count(&mut sim, player, row("RoadPass")), 1);
    assert_eq!(count(&mut sim, player, row("FishSteak")), 0);
    leave(&mut sim, 1, player);

    let bram = talk(&mut sim, 1, player, row("Bram"));
    assert!(!locked(&bram, "Ilsa gave me this pass."));
    pick(&mut sim, 1, player, "Ilsa gave me this pass.");
    let crossing = *sim
        .world()
        .get::<Crossing>(player)
        .expect("sailing for the forest");
    assert_eq!(crossing.dest_area, row("Forest"));
    assert_eq!(count(&mut sim, player, row("RoadPass")), 1);
}

#[test]
fn bram_sails_for_twenty_gold_and_the_crossing_is_narrated() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    heard_the_news(&mut sim, player);
    give(&mut sim, player, row("Gold"), 25);

    let bram = talk(&mut sim, 1, player, row("Bram"));
    assert!(!labels(&bram).contains(&"Ilsa gave me this pass.".to_owned()));
    pick(&mut sim, 1, player, "Sail to the forest.");

    assert_eq!(count(&mut sim, player, row("Gold")), 5);
    assert!(sim.world().get::<Crossing>(player).is_some());
    let told: Vec<Notification> = sim
        .notified(1)
        .into_iter()
        .map(|sent| sent.notification)
        .collect();
    assert!(told.contains(&spoken(row("GullSails"))));
}

#[test]
fn ilsa_has_a_word_only_for_friends_of_the_clan() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    heard_the_news(&mut sim, player);
    remember(&mut sim, player, row("SidedWithOrcs"));

    talk(&mut sim, 1, player, row("Ilsa"));
    let passed = pick(&mut sim, 1, player, "Ugra's clan walks with me.").expect("talking");
    assert_eq!(passed.node, row("IlsaUneasy"));
    assert_eq!(count(&mut sim, player, row("RoadPass")), 1);
}

#[test]
fn crossing_the_forest_road_on_the_way_elsewhere_is_no_business_of_ilsas() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    let (_, road) = forest_road(&sim);
    let before = Pos::new(road.center().x, road.center().y + 3.0);
    let beyond = Pos::new(road.center().x, road.center().y - 2.0);
    sim.world()
        .entity_mut(player)
        .insert(game::systems::movement::Position { pos: before });

    sim.send(1, MoveRequest { pos: beyond });
    assert!(sim.run_until(15.0, |world| {
        position(world, player).is_some_and(|at| at.distance(beyond) < Tiles(0.5))
    }));
    assert!(conversation(&mut sim, player).is_none());
}
