use bevy_ecs::prelude::*;
use game::core::tiling::Tiles;
use game::core::time::{Seconds, WallClock};
use game::data;
use game::data::dialogue::Id as DialogueId;
use game::data::item::Id as ItemId;
use game::data::memory::Id as MemoryId;
use game::data::npc::Id as NpcId;
use game::data::prop::Id as PropId;
use game::systems::area::MarkerName;
use game::systems::combat::Died;
use game::systems::dialogue::{Conversation, ConversationRequest};
use game::systems::interact::InteractRequest;
use game::systems::item::{
    self, DropItemRequest, DroppedItem, INVENTORY_MAX, Inventory, ItemStack, UseItemRequest,
};
use game::systems::memory::{self, Memory};
use game::systems::movement::{Position, position};
use game::systems::npc::{self, Npc, Pack};
use game::systems::player::Xp;
use game::systems::prop::Prop;
use game::systems::quest::{self, QuestId, QuestLog, QuestRequest, QuestResult};
use game::systems::shop::ShopRequest;

use crate::support::Sim;

fn townsperson(sim: &mut Sim, who: NpcId) -> Entity {
    let world = sim.world();
    world
        .query::<(Entity, &Npc)>()
        .iter(world)
        .find(|(_, npc)| npc.def == who)
        .map(|(entity, _)| entity)
        .expect("a resident")
}

fn prop(sim: &mut Sim, which: PropId) -> Entity {
    let world = sim.world();
    world
        .query::<(Entity, &Prop)>()
        .iter(world)
        .find(|(_, prop)| prop.def == which)
        .map(|(entity, _)| entity)
        .expect("a fixture on the map")
}

fn open_with(sim: &mut Sim, client: u32, player: Entity, target: Entity) -> Conversation {
    sim.send(client, InteractRequest { target });
    assert!(
        sim.run_until(20.0, |world| world.get::<Conversation>(player).is_some()),
        "the conversation never opened"
    );
    conversation(sim, player).expect("open")
}

fn talk(sim: &mut Sim, client: u32, player: Entity, who: NpcId) -> Conversation {
    let npc = townsperson(sim, who);
    let opened = open_with(sim, client, player, npc);
    assert_eq!(opened.with, Some(npc), "someone else spoke first");
    opened
}

fn heard_the_news(sim: &mut Sim, player: Entity) {
    let clock = *sim.world().resource::<WallClock>();
    sim.world()
        .get_mut::<Memory>(player)
        .expect("memory")
        .remember(MemoryId::TobbNewsToday, clock);
}

fn conversation(sim: &mut Sim, player: Entity) -> Option<Conversation> {
    sim.world().get::<Conversation>(player).cloned()
}

fn labels(conversation: &Conversation) -> Vec<String> {
    conversation
        .choices
        .iter()
        .map(|choice| choice.label.words())
        .collect()
}

fn pick(sim: &mut Sim, client: u32, player: Entity, label: &str) -> Option<Conversation> {
    let now = conversation(sim, player).expect("talking");
    let choice =
        now.choices
            .iter()
            .position(|choice| choice.label.words() == label)
            .unwrap_or_else(|| panic!("no choice {label:?} in {:?}", labels(&now))) as u32;
    sim.send(
        client,
        ConversationRequest::Pick {
            step: now.step,
            choice,
        },
    );
    sim.tick();
    conversation(sim, player)
}

fn refusal(conversation: &Conversation, label: &str) -> Option<String> {
    conversation
        .choices
        .iter()
        .find(|choice| choice.label.words() == label)
        .unwrap_or_else(|| panic!("no choice {label:?} in {:?}", labels(conversation)))
        .refusal
        .clone()
}

fn leave(sim: &mut Sim, client: u32, player: Entity) {
    if let Some(now) = conversation(sim, player) {
        sim.send(client, ConversationRequest::Leave { step: now.step });
        sim.tick();
    }
}

fn give(sim: &mut Sim, player: Entity, item: ItemId, count: u32) {
    sim.world()
        .get_mut::<Inventory>(player)
        .expect("bag")
        .exchange(&[], &[ItemStack::new(item, count)])
        .expect("room");
}

fn count(sim: &mut Sim, player: Entity, item: ItemId) -> u32 {
    sim.world()
        .get::<Inventory>(player)
        .expect("bag")
        .count(item)
}

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

fn settle(sim: &mut Sim) {
    sim.run_until(0.5, |_| false);
}

fn later(sim: &mut Sim, seconds: f32) {
    let clock = *sim.world().resource::<WallClock>();
    sim.world().insert_resource(WallClock {
        now: clock.now.after(Seconds(seconds)),
        ..clock
    });
    settle(sim);
}

fn slay(sim: &mut Sim, player: Entity, what: NpcId) {
    let world = sim.world();
    let at = position(world, player).expect("player position");
    let area = world.resource::<game::systems::WorldArea>().0;
    let victim = npc::spawn(world, what, at, area, Pack(u32::MAX));
    item::reserve(world, victim, player, Seconds(0.0));
    world.write_message(Died {
        entity: victim,
        killer: player,
    });
    sim.run_until(0.2, |_| false);
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
        give(sim, player, ItemId::RustySword, 1);
    }
}

#[test]
fn accepting_a_quest_logs_it_tracks_it_and_hands_over_its_items() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    heard_the_news(&mut sim, player);

    let greeting = talk(&mut sim, 1, player, NpcId::Tobb);
    assert!(labels(&greeting).contains(&"A Letter for the Captain".to_owned()));
    let offer = pick(&mut sim, 1, player, "A Letter for the Captain").expect("the offer");
    assert_eq!(offer.node, DialogueId::LetterOffer);
    pick(&mut sim, 1, player, "I'll take it.");

    let log = log(&mut sim, player);
    assert!(log.active(QuestId::LetterForTheCaptain).is_some());
    assert!(log.tracked.contains(&QuestId::LetterForTheCaptain));
    assert_eq!(count(&mut sim, player, ItemId::TobbsLetter), 1);
}

#[test]
fn abandoning_takes_the_quest_items_back_and_accepting_again_returns_them() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    heard_the_news(&mut sim, player);
    quest::accept(sim.world(), player, QuestId::LetterForTheCaptain);
    give(&mut sim, player, ItemId::TobbsLetter, 1);

    sim.send(
        1,
        QuestRequest::Abandon {
            quest: QuestId::LetterForTheCaptain,
        },
    );
    sim.tick();
    assert!(log(&mut sim, player).active.is_empty());
    assert_eq!(count(&mut sim, player, ItemId::TobbsLetter), 0);

    talk(&mut sim, 1, player, NpcId::Tobb);
    pick(&mut sim, 1, player, "A Letter for the Captain");
    pick(&mut sim, 1, player, "I'll take it.");
    assert_eq!(count(&mut sim, player, ItemId::TobbsLetter), 1);
}

#[test]
fn delivering_completes_the_quest_once_and_unlocks_the_next_in_the_chain() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    heard_the_news(&mut sim, player);
    talk(&mut sim, 1, player, NpcId::Tobb);
    pick(&mut sim, 1, player, "A Letter for the Captain");
    pick(&mut sim, 1, player, "I'll take it.");
    settle(&mut sim);
    level_up(&mut sim, player);
    let gold = count(&mut sim, player, ItemId::Gold);

    let bram = talk(&mut sim, 1, player, NpcId::Bram);
    assert!(refusal(&bram, "Bats in the Belfry").is_some());
    let thanks = pick(&mut sim, 1, player, "A Letter for the Captain").expect("thanks");
    assert_eq!(thanks.node, DialogueId::LetterThanks);
    let stale = thanks.step;
    pick(&mut sim, 1, player, "Tobb asked me to bring you this.");

    assert_eq!(count(&mut sim, player, ItemId::TobbsLetter), 0);
    assert_eq!(count(&mut sim, player, ItemId::Gold), gold + 10);
    let finished = log(&mut sim, player);
    assert_eq!(
        finished
            .finished(QuestId::LetterForTheCaptain)
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
    assert_eq!(count(&mut sim, player, ItemId::Gold), gold + 10);

    let again = talk(&mut sim, 1, player, NpcId::Bram);
    assert!(!labels(&again).contains(&"A Letter for the Captain".to_owned()));
    assert_eq!(refusal(&again, "Bats in the Belfry"), None);
}

#[test]
fn a_chain_stays_locked_until_its_previous_quest_and_level_are_met() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    heard_the_news(&mut sim, player);

    let bram = talk(&mut sim, 1, player, NpcId::Bram);
    assert!(refusal(&bram, "Bats in the Belfry").is_some());
    pick(&mut sim, 1, player, "Bats in the Belfry");
    assert!(log(&mut sim, player).active.is_empty());
}

#[test]
fn kills_count_only_for_the_credited_player() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let hunter = sim.join(1);
    let bystander = sim.join(2);
    quest::accept(sim.world(), hunter, QuestId::BatsInTheBelfry);
    quest::accept(sim.world(), bystander, QuestId::BatsInTheBelfry);

    slay(&mut sim, hunter, NpcId::VampireBat);
    slay(&mut sim, hunter, NpcId::Bat);

    let defeated = |log: QuestLog| {
        log.active(QuestId::BatsInTheBelfry)
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
            .filter(|dropped| dropped.item == ItemId::BelfryKey)
            .count()
    };

    for _ in 0..8 {
        slay(&mut sim, player, NpcId::VampireBat);
    }
    assert_eq!(keys(&mut sim), 0);

    quest::accept(sim.world(), player, QuestId::BatsInTheBelfry);
    for _ in 0..8 {
        slay(&mut sim, player, NpcId::VampireBat);
    }
    assert!(keys(&mut sim) >= 1);
}

#[test]
fn quest_items_never_sell_or_drop() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    heard_the_news(&mut sim, player);
    quest::accept(sim.world(), player, QuestId::LetterForTheCaptain);
    give(&mut sim, player, ItemId::TobbsLetter, 1);

    let slot = slot_of(&mut sim, player, ItemId::TobbsLetter);
    sim.send(1, DropItemRequest { slot });
    sim.tick();
    assert_eq!(count(&mut sim, player, ItemId::TobbsLetter), 1);

    talk(&mut sim, 1, player, NpcId::Mara);
    pick(&mut sim, 1, player, "Show me your wares.");
    let slot = slot_of(&mut sim, player, ItemId::TobbsLetter);
    sim.send(
        1,
        ShopRequest::Sell {
            slot,
            stack: ItemStack::new(ItemId::TobbsLetter, 1),
        },
    );
    sim.tick();
    assert_eq!(count(&mut sim, player, ItemId::TobbsLetter), 1);
}

#[test]
fn a_full_bag_refuses_the_reward_counting_space_after_the_hand_in() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    heard_the_news(&mut sim, player);
    quest::accept(sim.world(), player, QuestId::TusksForTheChief);
    slay(&mut sim, player, NpcId::OrcChief);
    give(&mut sim, player, ItemId::Gold, 1);
    give(&mut sim, player, ItemId::GreaterHealthPotion, 1);
    give(&mut sim, player, ItemId::OrcTusk, 7);
    fill_bag(&mut sim, player);
    settle(&mut sim);

    talk(&mut sim, 1, player, NpcId::Mara);
    let thanks = pick(&mut sim, 1, player, "Tusks for the Chief").expect("thanks");
    assert_eq!(thanks.node, DialogueId::TusksThanks);
    let refused = refusal(&thanks, "I'll take the Bone Shield.");
    assert_eq!(refused.as_deref(), Some("Needs 1 free slot"));
    pick(&mut sim, 1, player, "I'll take the Bone Shield.");
    assert_eq!(count(&mut sim, player, ItemId::OrcTusk), 7);
    assert_eq!(count(&mut sim, player, ItemId::BoneShield), 0);
    leave(&mut sim, 1, player);

    let tusk = slot_of(&mut sim, player, ItemId::OrcTusk);
    sim.send(1, DropItemRequest { slot: tusk });
    sim.tick();
    give(&mut sim, player, ItemId::OrcTusk, 5);
    settle(&mut sim);

    talk(&mut sim, 1, player, NpcId::Mara);
    pick(&mut sim, 1, player, "Tusks for the Chief");
    pick(&mut sim, 1, player, "I'll take the Bone Shield.");
    assert_eq!(count(&mut sim, player, ItemId::OrcTusk), 0);
    assert_eq!(count(&mut sim, player, ItemId::BoneShield), 1);
    assert_eq!(
        log(&mut sim, player)
            .finished(QuestId::TusksForTheChief)
            .map(|done| done.result),
        Some(QuestResult::Completed)
    );
}

#[test]
fn a_daily_quest_returns_after_the_reset() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    heard_the_news(&mut sim, player);
    quest::accept(sim.world(), player, QuestId::BoneTithe);
    give(&mut sim, player, ItemId::Bone, 10);
    settle(&mut sim);

    talk(&mut sim, 1, player, NpcId::Wren);
    pick(&mut sim, 1, player, "Bone Tithe");
    pick(&mut sim, 1, player, "Here are today's bones.");
    assert_eq!(count(&mut sim, player, ItemId::BoneToken), 6);

    let today = talk(&mut sim, 1, player, NpcId::Wren);
    assert!(!labels(&today).contains(&"Bone Tithe".to_owned()));
    leave(&mut sim, 1, player);

    later(&mut sim, 24.0 * 3600.0);
    let tomorrow = talk(&mut sim, 1, player, NpcId::Wren);
    assert!(labels(&tomorrow).contains(&"Bone Tithe".to_owned()));
}

#[test]
fn a_timed_quest_fails_when_time_runs_out_and_is_offered_again() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    heard_the_news(&mut sim, player);
    talk(&mut sim, 1, player, NpcId::Tobb);
    pick(&mut sim, 1, player, "Low Tide");
    pick(&mut sim, 1, player, "I'll hurry.");
    assert!(log(&mut sim, player).active(QuestId::LowTide).is_some());

    later(&mut sim, 601.0);

    let failed = log(&mut sim, player);
    assert!(failed.active(QuestId::LowTide).is_none());
    assert_eq!(
        failed.finished(QuestId::LowTide).map(|done| done.result),
        Some(QuestResult::Failed)
    );
    assert!(failed.tracked.contains(&QuestId::LowTide));
    let again = talk(&mut sim, 1, player, NpcId::Tobb);
    assert!(labels(&again).contains(&"Low Tide".to_owned()));
}

#[test]
fn an_item_starts_its_quest_and_offers_it_again_after_abandoning() {
    let mut sim = Sim::area(data::area::Id::Forest);
    let player = sim.join(1);
    give(&mut sim, player, ItemId::TatteredMap, 1);

    let slot = slot_of(&mut sim, player, ItemId::TatteredMap);
    sim.send(1, UseItemRequest { slot });
    sim.tick();
    let offer = conversation(&mut sim, player).expect("the map speaks");
    assert_eq!(offer.node, DialogueId::XMarksOffer);
    assert_eq!(offer.with, None);
    pick(&mut sim, 1, player, "Follow the map.");
    assert!(
        log(&mut sim, player)
            .active(QuestId::XMarksTheSpot)
            .is_some()
    );
    assert_eq!(count(&mut sim, player, ItemId::TatteredMap), 1);

    sim.send(
        1,
        QuestRequest::Abandon {
            quest: QuestId::XMarksTheSpot,
        },
    );
    sim.tick();
    sim.send(1, UseItemRequest { slot });
    sim.tick();
    assert_eq!(
        conversation(&mut sim, player).map(|now| now.node),
        Some(DialogueId::XMarksOffer)
    );
}

#[test]
fn exploring_the_marked_spot_readies_the_quest_for_the_object_there() {
    let mut sim = Sim::area(data::area::Id::Forest);
    let player = sim.join(1);
    give(&mut sim, player, ItemId::TatteredMap, 1);
    quest::accept(sim.world(), player, QuestId::XMarksTheSpot);
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

    let target = prop(&mut sim, PropId::StandingStone);
    open_with(&mut sim, 1, player, target);
    pick(&mut sim, 1, player, "X Marks the Spot");
    pick(&mut sim, 1, player, "Dig where the cross says.");
    assert_eq!(count(&mut sim, player, ItemId::TatteredMap), 0);
    assert_eq!(count(&mut sim, player, ItemId::CorsairCutlass), 1);
}

#[test]
fn giving_ugra_the_tusks_fails_maras_quest_and_is_remembered() {
    let mut sim = Sim::area(data::area::Id::Forest);
    let player = sim.join(1);
    quest::accept(sim.world(), player, QuestId::TusksForTheChief);
    give(&mut sim, player, ItemId::OrcTusk, 5);
    settle(&mut sim);

    talk(&mut sim, 1, player, NpcId::Ugra);
    pick(&mut sim, 1, player, "The Shaman's Plea");
    let answer = pick(&mut sim, 1, player, "I'm listening.").expect("her plea");
    assert_eq!(answer.node, DialogueId::PleaAnswer);
    let given = pick(&mut sim, 1, player, "Give her the tusks.").expect("her thanks");
    assert_eq!(given.node, DialogueId::UgraGrateful);

    let answered = log(&mut sim, player);
    assert_eq!(
        answered
            .finished(QuestId::TusksForTheChief)
            .map(|done| done.result),
        Some(QuestResult::Failed)
    );
    assert_eq!(
        answered
            .finished(QuestId::ShamansPlea)
            .map(|done| done.result),
        Some(QuestResult::Completed)
    );
    assert_eq!(count(&mut sim, player, ItemId::OrcTusk), 0);
    assert!(memory::recall(sim.world(), player, MemoryId::SidedWithOrcs).is_some());
    leave(&mut sim, 1, player);
    settle(&mut sim);
    let settled = log(&mut sim, player);
    assert!(!settled.trackable(QuestId::TusksForTheChief));
    assert!(!settled.tracked.contains(&QuestId::TusksForTheChief));

    let friend = talk(&mut sim, 1, player, NpcId::Ugra);
    assert_eq!(friend.node, DialogueId::UgraFriend);
    assert!(labels(&friend).contains(&"Rest for the Fallen".to_owned()));
}
