use bevy_ecs::prelude::*;
use game::core::tiling::Tiles;
use game::data;
use game::data::item::Id as ItemId;
use game::systems::area::MarkerName;
use game::systems::dialogue::ConversationRequest;
use game::systems::item::{
    DropItemRequest, DroppedItem, INVENTORY_MAX, Inventory, ItemStack, UseItemRequest,
};
use game::systems::memory::{self};
use game::systems::movement::Position;
use game::systems::player::Xp;
use game::systems::quest::{self, QuestLog, QuestRequest, QuestResult};
use game::systems::shop::ShopRequest;

use crate::support::{
    Sim, content, conversation, count, errors, give, heard_the_news, labels, later, leave, locked,
    open_with, pick, prop, row, settle, slay, talk,
};

fn slot_of(sim: &mut Sim, player: Entity, item: ItemId) -> u32 {
    sim.world()
        .get::<Inventory>(player)
        .expect("bag")
        .slots
        .iter()
        .position(|stack| stack.item == item)
        .expect("in the bag") as u32
}

fn log(sim: &mut Sim, player: Entity) -> QuestLog {
    sim.world()
        .get::<QuestLog>(player)
        .cloned()
        .expect("a quest log")
}

fn level_up(sim: &mut Sim, player: Entity) {
    sim.world().get_mut::<Xp>(player).expect("xp").gain(100_000);
}

fn fill_bag(sim: &mut Sim, player: Entity) {
    let free = INVENTORY_MAX as usize
        - sim
            .world()
            .get::<Inventory>(player)
            .expect("bag")
            .slots
            .len();
    for _ in 0..free {
        give(sim, player, row("RustySword"), 1);
    }
}

#[test]
fn accepting_a_quest_logs_it_tracks_it_and_hands_over_its_items() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    heard_the_news(&mut sim, player);

    let greeting = talk(&mut sim, 1, player, row("Tobb"));
    assert!(labels(&greeting).contains(&"A Letter for the Captain".to_owned()));
    let offer = pick(&mut sim, 1, player, "A Letter for the Captain").expect("the offer");
    assert_eq!(offer.node, row("LetterOffer"));
    pick(&mut sim, 1, player, "I'll take it.");

    let log = log(&mut sim, player);
    assert!(log.active(row("LetterForTheCaptain")).is_some());
    assert!(log.tracked.contains(&row("LetterForTheCaptain")));
    assert_eq!(count(&mut sim, player, row("TobbsLetter")), 1);
}

#[test]
fn abandoning_takes_the_quest_items_back_and_accepting_again_returns_them() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    heard_the_news(&mut sim, player);
    quest::accept(sim.world(), player, row("LetterForTheCaptain"));
    give(&mut sim, player, row("TobbsLetter"), 1);

    sim.send(
        1,
        QuestRequest::Abandon {
            quest: row("LetterForTheCaptain"),
        },
    );
    sim.tick();
    assert!(log(&mut sim, player).active.is_empty());
    assert_eq!(count(&mut sim, player, row("TobbsLetter")), 0);

    talk(&mut sim, 1, player, row("Tobb"));
    pick(&mut sim, 1, player, "A Letter for the Captain");
    pick(&mut sim, 1, player, "I'll take it.");
    assert_eq!(count(&mut sim, player, row("TobbsLetter")), 1);
}

#[test]
fn delivering_completes_the_quest_once_and_unlocks_the_next_in_the_chain() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    heard_the_news(&mut sim, player);
    talk(&mut sim, 1, player, row("Tobb"));
    pick(&mut sim, 1, player, "A Letter for the Captain");
    pick(&mut sim, 1, player, "I'll take it.");
    settle(&mut sim);
    level_up(&mut sim, player);
    let gold = count(&mut sim, player, row("Gold"));

    let bram = talk(&mut sim, 1, player, row("Bram"));
    assert!(!labels(&bram).contains(&"Bats in the Belfry".to_owned()));
    let thanks = pick(&mut sim, 1, player, "A Letter for the Captain").expect("thanks");
    assert_eq!(thanks.node, row("LetterThanks"));
    let stale = thanks.step;
    pick(&mut sim, 1, player, "Tobb asked me to bring you this.");

    assert_eq!(count(&mut sim, player, row("TobbsLetter")), 0);
    assert_eq!(count(&mut sim, player, row("Gold")), gold + 10);
    let finished = log(&mut sim, player);
    assert_eq!(
        finished
            .finished(row("LetterForTheCaptain"))
            .map(|done| done.result),
        Some(QuestResult::Completed)
    );

    sim.send(
        1,
        ConversationRequest::Pick {
            step: stale,
            choice: 0,
        },
    );
    sim.tick();
    assert_eq!(count(&mut sim, player, row("Gold")), gold + 10);

    let again = talk(&mut sim, 1, player, row("Bram"));
    assert!(!labels(&again).contains(&"A Letter for the Captain".to_owned()));
    assert!(!locked(&again, "Bats in the Belfry"));
}

#[test]
fn a_chain_stays_hidden_until_its_previous_quest_and_level_are_met() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    heard_the_news(&mut sim, player);
    level_up(&mut sim, player);

    let bram = talk(&mut sim, 1, player, row("Bram"));
    assert!(!labels(&bram).contains(&"Bats in the Belfry".to_owned()));
}

#[test]
fn kills_count_only_for_the_credited_player() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let hunter = sim.join(1);
    let bystander = sim.join(2);
    quest::accept(sim.world(), hunter, row("BatsInTheBelfry"));
    quest::accept(sim.world(), bystander, row("BatsInTheBelfry"));

    slay(&mut sim, hunter, row("VampireBat"));
    slay(&mut sim, hunter, row("Bat"));

    let defeated = |log: QuestLog| {
        log.active(row("BatsInTheBelfry"))
            .map(|active| active.counts[0])
    };
    assert_eq!(defeated(log(&mut sim, hunter)), Some(1));
    assert_eq!(defeated(log(&mut sim, bystander)), Some(0));
}

#[test]
fn quest_drops_fall_only_for_players_on_the_quest() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    let keys = |sim: &mut Sim| {
        let world = sim.world();
        world
            .query::<&DroppedItem>()
            .iter(world)
            .filter(|dropped| dropped.item == row("BelfryKey"))
            .count()
    };

    for _ in 0..8 {
        slay(&mut sim, player, row("VampireBat"));
    }
    assert_eq!(keys(&mut sim), 0);

    quest::accept(sim.world(), player, row("BatsInTheBelfry"));
    for _ in 0..8 {
        slay(&mut sim, player, row("VampireBat"));
    }
    assert!(keys(&mut sim) >= 1);
}

#[test]
fn quest_items_never_sell_or_drop() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    heard_the_news(&mut sim, player);
    quest::accept(sim.world(), player, row("LetterForTheCaptain"));
    give(&mut sim, player, row("TobbsLetter"), 1);

    let slot = slot_of(&mut sim, player, row("TobbsLetter"));
    sim.send(1, DropItemRequest { slot });
    sim.tick();
    assert_eq!(count(&mut sim, player, row("TobbsLetter")), 1);

    talk(&mut sim, 1, player, row("Mara"));
    pick(&mut sim, 1, player, "Show me your wares.");
    let slot = slot_of(&mut sim, player, row("TobbsLetter"));
    sim.send(
        1,
        ShopRequest::Sell {
            slot,
            stack: ItemStack::new(row("TobbsLetter"), 1),
        },
    );
    sim.tick();
    assert_eq!(count(&mut sim, player, row("TobbsLetter")), 1);
}

#[test]
fn a_full_bag_refuses_the_reward_counting_space_after_the_hand_in() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    heard_the_news(&mut sim, player);
    quest::accept(sim.world(), player, row("TusksForTheChief"));
    slay(&mut sim, player, row("OrcChief"));
    give(&mut sim, player, row("Gold"), 1);
    give(&mut sim, player, row("GreaterHealthPotion"), 1);
    give(&mut sim, player, row("OrcTusk"), 7);
    fill_bag(&mut sim, player);
    settle(&mut sim);

    talk(&mut sim, 1, player, row("Mara"));
    let thanks = pick(&mut sim, 1, player, "Tusks for the Chief").expect("thanks");
    assert_eq!(thanks.node, row("TusksThanks"));
    assert!(!locked(&thanks, "I'll take the Bone Shield."));
    let refused =
        pick(&mut sim, 1, player, "I'll take the Bone Shield.").and_then(|now| now.refused);
    assert!(refused.is_some());
    assert_eq!(
        errors(&sim, 1).last().map(String::as_str),
        Some("Needs 1 free slot")
    );
    assert_eq!(count(&mut sim, player, row("OrcTusk")), 7);
    assert_eq!(count(&mut sim, player, row("BoneShield")), 0);
    leave(&mut sim, 1, player);

    let tusk = slot_of(&mut sim, player, row("OrcTusk"));
    sim.send(1, DropItemRequest { slot: tusk });
    sim.tick();
    give(&mut sim, player, row("OrcTusk"), 5);
    settle(&mut sim);

    talk(&mut sim, 1, player, row("Mara"));
    pick(&mut sim, 1, player, "Tusks for the Chief");
    pick(&mut sim, 1, player, "I'll take the Bone Shield.");
    assert_eq!(count(&mut sim, player, row("OrcTusk")), 0);
    assert_eq!(count(&mut sim, player, row("BoneShield")), 1);
    assert_eq!(
        log(&mut sim, player)
            .finished(row("TusksForTheChief"))
            .map(|done| done.result),
        Some(QuestResult::Completed)
    );
}

#[test]
fn a_daily_quest_returns_after_the_reset() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    heard_the_news(&mut sim, player);
    quest::accept(sim.world(), player, row("BoneTithe"));
    give(&mut sim, player, row("Bone"), 10);
    settle(&mut sim);

    talk(&mut sim, 1, player, row("Wren"));
    pick(&mut sim, 1, player, "Bone Tithe");
    pick(&mut sim, 1, player, "Here are today's bones.");
    assert_eq!(count(&mut sim, player, row("BoneToken")), 6);

    let today = talk(&mut sim, 1, player, row("Wren"));
    assert!(!labels(&today).contains(&"Bone Tithe".to_owned()));
    leave(&mut sim, 1, player);

    later(&mut sim, 24.0 * 3600.0);
    let tomorrow = talk(&mut sim, 1, player, row("Wren"));
    assert!(labels(&tomorrow).contains(&"Bone Tithe".to_owned()));
}

#[test]
fn a_timed_quest_fails_when_time_runs_out_and_is_offered_again() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    heard_the_news(&mut sim, player);
    talk(&mut sim, 1, player, row("Tobb"));
    pick(&mut sim, 1, player, "Low Tide");
    pick(&mut sim, 1, player, "I'll hurry.");
    assert!(log(&mut sim, player).active(row("LowTide")).is_some());

    later(&mut sim, 601.0);

    let failed = log(&mut sim, player);
    assert!(failed.active(row("LowTide")).is_none());
    assert_eq!(
        failed.finished(row("LowTide")).map(|done| done.result),
        Some(QuestResult::Failed)
    );
    assert!(failed.tracked.contains(&row("LowTide")));
    let again = talk(&mut sim, 1, player, row("Tobb"));
    assert!(labels(&again).contains(&"Low Tide".to_owned()));
}

#[test]
fn an_item_starts_its_quest_and_offers_it_again_after_abandoning() {
    let mut sim = Sim::area(row("Forest"));
    let player = sim.join(1);
    give(&mut sim, player, row("TatteredMap"), 1);

    let slot = slot_of(&mut sim, player, row("TatteredMap"));
    sim.send(1, UseItemRequest { slot });
    sim.tick();
    let offer = conversation(&mut sim, player).expect("the map speaks");
    assert_eq!(offer.node, row("XMarksOffer"));
    assert_eq!(offer.with, None);
    pick(&mut sim, 1, player, "Follow the map.");
    assert!(log(&mut sim, player).active(row("XMarksTheSpot")).is_some());
    assert_eq!(count(&mut sim, player, row("TatteredMap")), 1);

    sim.send(
        1,
        QuestRequest::Abandon {
            quest: row("XMarksTheSpot"),
        },
    );
    sim.tick();
    sim.send(1, UseItemRequest { slot });
    sim.tick();
    assert_eq!(
        conversation(&mut sim, player).map(|now| now.node),
        Some(row("XMarksOffer"))
    );
}

#[test]
fn exploring_the_marked_spot_readies_the_quest_for_the_object_there() {
    let mut sim = Sim::area(row("Forest"));
    let player = sim.join(1);
    give(&mut sim, player, row("TatteredMap"), 1);
    quest::accept(sim.world(), player, row("XMarksTheSpot"));
    settle(&mut sim);
    assert!(!log(&mut sim, player).active[0].ready);

    let stone = sim
        .map()
        .marker(MarkerName("standing-stone"))
        .expect("the forest marks its standing stone")
        .center();
    let near = sim.walkable_near(stone, Tiles(1.0), Tiles(1.5));
    sim.world()
        .entity_mut(player)
        .insert(Position { pos: near });
    settle(&mut sim);
    assert!(log(&mut sim, player).active[0].ready);

    let target = prop(&mut sim, row("StandingStone"));
    open_with(&mut sim, 1, player, target);
    pick(&mut sim, 1, player, "X Marks the Spot");
    pick(&mut sim, 1, player, "Dig where the cross says.");
    assert_eq!(count(&mut sim, player, row("TatteredMap")), 0);
    assert_eq!(count(&mut sim, player, row("CorsairCutlass")), 1);
}

#[test]
fn giving_ugra_the_tusks_fails_maras_quest_and_is_remembered() {
    let mut sim = Sim::area(row("Forest"));
    let player = sim.join(1);
    quest::accept(sim.world(), player, row("TusksForTheChief"));
    give(&mut sim, player, row("OrcTusk"), 5);
    settle(&mut sim);

    talk(&mut sim, 1, player, row("Ugra"));
    pick(&mut sim, 1, player, "The Shaman's Plea");
    let answer = pick(&mut sim, 1, player, "I'm listening.").expect("her plea");
    assert_eq!(answer.node, row("PleaAnswer"));
    let given = pick(&mut sim, 1, player, "Give her the tusks.").expect("her thanks");
    assert_eq!(given.node, row("UgraGrateful"));

    let answered = log(&mut sim, player);
    assert_eq!(
        answered
            .finished(row("TusksForTheChief"))
            .map(|done| done.result),
        Some(QuestResult::Failed)
    );
    assert_eq!(
        answered
            .finished(row("ShamansPlea"))
            .map(|done| done.result),
        Some(QuestResult::Completed)
    );
    assert_eq!(count(&mut sim, player, row("OrcTusk")), 0);
    assert!(memory::recall(sim.world(), player, row("SidedWithOrcs")).is_some());
    leave(&mut sim, 1, player);
    settle(&mut sim);
    let settled = log(&mut sim, player);
    assert!(!settled.trackable(content(), row("TusksForTheChief")));
    assert!(!settled.tracked.contains(&row("TusksForTheChief")));

    let friend = talk(&mut sim, 1, player, row("Ugra"));
    assert_eq!(friend.node, row("UgraFriend"));
    assert!(labels(&friend).contains(&"Rest for the Fallen".to_owned()));
}
