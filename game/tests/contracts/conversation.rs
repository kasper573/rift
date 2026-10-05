use bevy_ecs::prelude::*;
use game::core::math::Pos;
use game::core::tiling::{TilePos, Tiles};
use game::core::time::{Seconds, UnixMillis, UtcHour, WallClock};
use game::data;
use game::data::attention::Id as AttentionId;
use game::data::dialogue::Id as DialogueId;
use game::data::item::Id as ItemId;
use game::data::memory::Id as MemoryId;
use game::systems::attention::Attention;
use game::systems::dialogue::{self, BusyPolicy, Conversation, ConversationRequest, Start};
use game::systems::input::map::InputMap;
use game::systems::interact::InteractRequest;
use game::systems::item::{Inventory, ItemStack};
use game::systems::memory::{self, Memory};
use game::systems::movement::{MoveRequest, Position, position};
use game::systems::npc::Npc;
use game::systems::player::commands_locked;
use game::systems::stat::{self, StatKind};

use crate::support::Sim;

fn townsperson(sim: &mut Sim, who: data::npc::Id) -> Entity {
    let world = sim.world();
    world
        .query::<(Entity, &Npc)>()
        .iter(world)
        .find(|(_, npc)| npc.def == who)
        .map(|(entity, _)| entity)
        .expect("a resident")
}

fn conversation(sim: &mut Sim, player: Entity) -> Option<Conversation> {
    sim.world().get::<Conversation>(player).cloned()
}

fn talk(sim: &mut Sim, client: u32, player: Entity, who: data::npc::Id) -> Conversation {
    let npc = townsperson(sim, who);
    sim.send(client, InteractRequest { target: npc });
    assert!(
        sim.run_until(20.0, |world| world.get::<Conversation>(player).is_some()),
        "the conversation with {who:?} never opened"
    );
    conversation(sim, player).expect("open")
}

fn choice(conversation: &Conversation, label: &str) -> u32 {
    conversation
        .choices
        .iter()
        .position(|choice| choice.label.words(&InputMap::default()) == label)
        .unwrap_or_else(|| panic!("no choice {label:?}")) as u32
}

fn give_gold(sim: &mut Sim, player: Entity, count: u32) {
    sim.world()
        .get_mut::<Inventory>(player)
        .expect("bag")
        .exchange(&[], &[ItemStack::new(ItemId::Gold, count)])
        .expect("room for gold");
}

fn gold(sim: &mut Sim, player: Entity) -> u32 {
    sim.world()
        .get::<Inventory>(player)
        .expect("bag")
        .count(ItemId::Gold)
}

fn start(node: DialogueId, busy: BusyPolicy) -> Start {
    Start {
        node,
        with: None,
        tether: None,
        requires: Vec::new(),
        busy,
    }
}

fn place(sim: &mut Sim, player: Entity, at: Pos<Tiles>) {
    sim.world().entity_mut(player).insert(Position { pos: at });
}

#[test]
fn talking_walks_into_reach_opens_the_greeting_and_locks_commands() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    let grisha = townsperson(&mut sim, data::npc::Id::Grisha);

    let opened = talk(&mut sim, 1, player, data::npc::Id::Grisha);

    assert_eq!(opened.node, DialogueId::GrishaHello);
    let at = position(sim.world(), player).expect("position");
    let grisha_at = position(sim.world(), grisha).expect("position");
    assert!(at.distance(grisha_at) <= Tiles(2.0 + std::f32::consts::SQRT_2));
    assert!(commands_locked(sim.world(), player));

    let elsewhere = sim.walkable_near(at, Tiles(4.0), Tiles(6.0));
    sim.send(1, MoveRequest { pos: elsewhere });
    sim.run_until(2.0, |_| false);
    let still = position(sim.world(), player).expect("position");
    assert!(still.distance(at) < Tiles(0.5));

    sim.send(1, ConversationRequest::Leave { step: opened.step });
    sim.tick();
    assert!(conversation(&mut sim, player).is_none());
    assert!(!commands_locked(sim.world(), player));
}

#[test]
fn a_pick_answers_its_step_once() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    give_gold(&mut sim, player, 30);
    let opened = talk(&mut sim, 1, player, data::npc::Id::Grisha);
    let round = ConversationRequest::Pick {
        step: opened.step,
        choice: choice(&opened, "Buy a round for the room."),
    };

    sim.send(1, round);
    sim.send(1, round);
    sim.tick();

    assert_eq!(gold(&mut sim, player), 20);
    let now = conversation(&mut sim, player).expect("still talking");
    assert_eq!(now.node, DialogueId::GrishaRound);
    assert_eq!(
        memory::recall(sim.world(), player, MemoryId::InnFavour),
        Some(1)
    );

    sim.send(1, round);
    sim.tick();
    assert_eq!(gold(&mut sim, player), 20);
}

#[test]
fn a_choice_that_costs_too_much_changes_nothing() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    let opened = talk(&mut sim, 1, player, data::npc::Id::Grisha);
    let index = choice(&opened, "Buy a round for the room.");
    assert!(opened.choices[index as usize].refusal.is_some());

    sim.send(
        1,
        ConversationRequest::Pick {
            step: opened.step,
            choice: index,
        },
    );
    sim.tick();

    let now = conversation(&mut sim, player).expect("still talking");
    assert_eq!(now.node, DialogueId::GrishaHello);
    assert_eq!(
        memory::recall(sim.world(), player, MemoryId::InnFavour),
        None
    );
}

#[test]
fn walking_out_of_reach_ends_the_conversation() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    talk(&mut sim, 1, player, data::npc::Id::Grisha);
    let at = position(sim.world(), player).expect("position");
    let away = sim.walkable_near(at, Tiles(6.0), Tiles(9.0));

    place(&mut sim, player, away);
    sim.tick();

    assert!(conversation(&mut sim, player).is_none());
    assert!(!commands_locked(sim.world(), player));
}

#[test]
fn damage_keeps_a_conversation_but_death_ends_it() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    talk(&mut sim, 1, player, data::npc::Id::Grisha);

    stat::apply_damage(sim.world(), player, 1.0);
    sim.tick();
    assert!(conversation(&mut sim, player).is_some());

    let health = stat::effective(sim.world(), player, StatKind::Health);
    stat::apply_damage(sim.world(), player, health);
    sim.tick();
    assert!(conversation(&mut sim, player).is_none());
}

#[test]
fn a_busy_player_waits_skips_or_is_replaced_as_each_start_says() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    let opened = talk(&mut sim, 1, player, data::npc::Id::Grisha);

    dialogue::start(
        sim.world(),
        player,
        start(DialogueId::BramHello, BusyPolicy::Skip),
    );
    dialogue::start(
        sim.world(),
        player,
        start(DialogueId::TobbNews, BusyPolicy::Wait(Seconds(30.0))),
    );
    sim.tick();
    let waiting = conversation(&mut sim, player).expect("talking");
    assert_eq!(waiting.node, DialogueId::GrishaHello);
    assert_eq!(
        waiting.waiting.map(|next| next.node),
        Some(DialogueId::TobbNews)
    );

    sim.send(1, ConversationRequest::Leave { step: opened.step });
    sim.tick();
    let next = conversation(&mut sim, player).expect("the waiting start began");
    assert_eq!(next.node, DialogueId::TobbNews);

    dialogue::start(
        sim.world(),
        player,
        start(DialogueId::IlsaHello, BusyPolicy::Replace),
    );
    sim.tick();
    assert_eq!(
        conversation(&mut sim, player).map(|now| now.node),
        Some(DialogueId::IlsaHello)
    );
}

#[test]
fn a_waiting_start_expires() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    let opened = talk(&mut sim, 1, player, data::npc::Id::Grisha);
    dialogue::start(
        sim.world(),
        player,
        start(DialogueId::TobbNews, BusyPolicy::Wait(Seconds(1.0))),
    );

    sim.run_until(2.0, |_| false);
    sim.send(1, ConversationRequest::Leave { step: opened.step });
    sim.tick();

    assert!(conversation(&mut sim, player).is_none());
}

#[test]
fn tobb_calls_over_a_visitor_he_has_news_for() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let player = sim.join(1);
    let opened = talk(&mut sim, 1, player, data::npc::Id::Tobb);
    assert_eq!(opened.node, DialogueId::TobbHello);
    assert_eq!(opened.waiting, None);

    assert!(sim.run_until(1.0, |world| {
        world
            .get::<Conversation>(player)
            .and_then(|now| now.waiting)
            .is_some_and(|next| next.node == DialogueId::TobbNews)
    }));
    sim.send(1, ConversationRequest::Leave { step: opened.step });
    sim.tick();
    let news = conversation(&mut sim, player).expect("Tobb's news");
    assert_eq!(news.node, DialogueId::TobbNews);

    sim.send(1, ConversationRequest::Leave { step: news.step });
    sim.run_until(2.0, |_| false);
    assert!(conversation(&mut sim, player).is_none());
}

#[test]
fn two_players_see_their_own_icons_over_the_same_npc() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let regular = sim.join(1);
    let stranger = sim.join(2);
    let tobb = townsperson(&mut sim, data::npc::Id::Tobb);
    for visit in 0..3 {
        let clock = WallClock {
            now: UnixMillis(visit * 3_600_000),
            reset: UtcHour::MIDNIGHT,
        };
        sim.world()
            .get_mut::<Memory>(regular)
            .expect("memory")
            .remember(MemoryId::TobbVisits, clock);
    }
    sim.world().insert_resource(WallClock {
        now: UnixMillis(3 * 3_600_000),
        reset: UtcHour::MIDNIGHT,
    });

    let marks = |sim: &mut Sim, player: Entity| -> Vec<AttentionId> {
        sim.world()
            .get::<Attention>(player)
            .map(|attention| attention.of(tobb).to_vec())
            .unwrap_or_default()
    };
    sim.run_until(0.5, |_| false);

    assert!(marks(&mut sim, regular).contains(&AttentionId::News));
    assert!(!marks(&mut sim, stranger).contains(&AttentionId::News));
}
