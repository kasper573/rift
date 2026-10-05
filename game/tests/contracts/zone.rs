use bevy_ecs::prelude::*;
use game::core::math::Pos;
use game::core::tiling::{TilePos, Tiles};
use game::data;
use game::data::announcement::Id as AnnouncementId;
use game::data::dialogue::Id as DialogueId;
use game::data::item::Id as ItemId;
use game::data::memory::Id as MemoryId;
use game::data::npc::Id as NpcId;
use game::systems::area::MarkerName;
use game::systems::area::transition::Crossing;
use game::systems::dialogue::Conversation;
use game::systems::dialogue::announcement::Announcements;
use game::systems::memory::Memory;
use game::systems::movement::{MoveRequest, MoveToPortal, position};
use game::systems::player::commands_locked;

use crate::support::{
    Sim, conversation, count, give, heard_the_news, labels, leave, pick, refusal, settle, talk,
    townsperson,
};

fn forest_road(sim: &Sim) -> (u32, Pos<Tiles>) {
    let map = sim.map();
    let gate = map
        .marker(MarkerName("forest-gate"))
        .expect("a gate on the map");
    let (index, portal) = map
        .portals
        .iter()
        .enumerate()
        .find(|(_, portal)| gate.covers(portal.rect.center()))
        .expect("a warp inside the gate");
    (index as u32, portal.rect.center())
}

fn head_for_the_forest_road(sim: &mut Sim, client: u32) {
    let (portal, pos) = forest_road(sim);
    sim.send(client, MoveToPortal { pos, portal });
}

fn remember(sim: &mut Sim, player: Entity, key: MemoryId) {
    let clock = *sim.world().resource::<game::core::time::WallClock>();
    sim.world()
        .get_mut::<Memory>(player)
        .expect("memory")
        .remember(key, clock);
}

#[test]
fn heading_for_the_forest_road_gets_ilsa_to_halt_you_once() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    let ilsa = townsperson(&mut sim, NpcId::Ilsa);

    head_for_the_forest_road(&mut sim, 1);
    assert!(
        sim.run_until(15.0, |world| world.get::<Conversation>(player).is_some()),
        "Ilsa never stopped you"
    );
    let halted = conversation(&mut sim, player).expect("halted");
    assert_eq!(halted.node, DialogueId::IlsaHalt);
    assert_eq!(halted.with, Some(ilsa));
    assert!(commands_locked(sim.world(), player));
    assert!(refusal(&halted, "I have a Road Pass.").is_some());

    leave(&mut sim, 1, player);
    head_for_the_forest_road(&mut sim, 1);
    assert!(
        sim.run_until(15.0, |world| world.get::<Crossing>(player).is_some()),
        "the warp never took you"
    );
    assert!(conversation(&mut sim, player).is_none());
}

#[test]
fn turning_toward_the_road_while_standing_by_it_still_gets_you_halted() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    let (_, warp) = forest_road(&sim);
    let beside = Pos::new(warp.x - 2.0, warp.y);
    sim.world()
        .entity_mut(player)
        .insert(game::systems::movement::Position { pos: beside });
    settle(&mut sim);
    assert!(conversation(&mut sim, player).is_none());

    head_for_the_forest_road(&mut sim, 1);
    assert!(
        sim.run_until(15.0, |world| world.get::<Conversation>(player).is_some()),
        "Ilsa let you through"
    );
    assert_eq!(
        conversation(&mut sim, player).map(|now| now.node),
        Some(DialogueId::IlsaHalt)
    );
}

#[test]
fn walking_past_the_forest_road_is_no_business_of_ilsas() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    let (_, warp) = forest_road(&sim);
    let past = Pos::new(warp.x - 2.0, warp.y - 1.0);

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
    give(&mut sim, player, ItemId::FishSteak, 1);

    let greeting = talk(&mut sim, 1, player, NpcId::Ilsa);
    assert!(!labels(&greeting).contains(&"Ugra's clan walks with me.".to_owned()));
    let traded = pick(&mut sim, 1, player, "Would a fish change your mind?").expect("talking");
    assert_eq!(traded.node, DialogueId::IlsaFish);
    assert_eq!(count(&mut sim, player, ItemId::RoadPass), 1);
    assert_eq!(count(&mut sim, player, ItemId::FishSteak), 0);
    leave(&mut sim, 1, player);

    let bram = talk(&mut sim, 1, player, NpcId::Bram);
    assert_eq!(refusal(&bram, "Ilsa gave me this pass."), None);
    pick(&mut sim, 1, player, "Ilsa gave me this pass.");
    let crossing = *sim
        .world()
        .get::<Crossing>(player)
        .expect("sailing for the forest");
    assert_eq!(crossing.dest_area, data::area::Id::Forest);
    assert_eq!(count(&mut sim, player, ItemId::RoadPass), 1);
}

#[test]
fn bram_sails_for_twenty_gold_and_the_crossing_is_announced() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    heard_the_news(&mut sim, player);
    give(&mut sim, player, ItemId::Gold, 25);

    let bram = talk(&mut sim, 1, player, NpcId::Bram);
    assert!(refusal(&bram, "Ilsa gave me this pass.").is_some());
    pick(&mut sim, 1, player, "Sail to the forest.");

    assert_eq!(count(&mut sim, player, ItemId::Gold), 5);
    assert!(sim.world().get::<Crossing>(player).is_some());
    let lane = sim
        .world()
        .get::<Announcements>(player)
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        lane.showing.map(|shown| shown.id),
        Some(AnnouncementId::GullSails)
    );
}

#[test]
fn ilsa_has_a_word_only_for_friends_of_the_clan() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    heard_the_news(&mut sim, player);
    remember(&mut sim, player, MemoryId::SidedWithOrcs);

    talk(&mut sim, 1, player, NpcId::Ilsa);
    let passed = pick(&mut sim, 1, player, "Ugra's clan walks with me.").expect("talking");
    assert_eq!(passed.node, DialogueId::IlsaUneasy);
    assert_eq!(count(&mut sim, player, ItemId::RoadPass), 1);
}
