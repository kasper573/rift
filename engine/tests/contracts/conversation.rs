use bevy_ecs::prelude::*;
use engine::core::content::Fixture;
use engine::core::geometry::Radians;
use engine::core::math::{Offset, Pos};
use engine::core::script::Value as Recalled;
use engine::core::time::{UnixMillis, UtcHour, WallClock};
use engine::core::units::{MeterPos, Meters};
use engine::systems::actor::{self, Action, Actor};
use engine::systems::attention::{self, Attention, AttentionId, AttentionMark};
use engine::systems::dialogue::{self, Conversation, ConversationRequest};
use engine::systems::interact::{self, InteractRequest};
use engine::systems::memory::{self, MemoryDef};
use engine::systems::movement::{self, MoveRequest, MoveSeq, moving, position};
use engine::systems::player::commands_locked;
use engine::systems::script::{self, EventKind, RiftInstances};
use engine::systems::stat::{self, Immortal, StatKind};
use engine::systems::{combat, npc};
use serde_json::{Value, json};
use testkit::{Game, Map, Sim, lines};

#[test]
fn a_conversation_reaches_the_client_as_the_world_holds_it() {
    let mut sim = inn();
    sim.join(1);

    let opened = sim.talk(1, "keeper");
    let wire = postcard::to_allocvec(&opened).expect("a conversation serializes");
    let received: Conversation = postcard::from_bytes(&wire).expect("a conversation deserializes");

    assert!(opened.lines.iter().any(|line| line.face.is_some()));
    assert_eq!(received, opened);
}

#[test]
fn talking_walks_into_reach_opens_the_greeting_and_locks_commands() {
    let mut sim = inn();
    let player = sim.join(1);
    let keeper = sim.npc("keeper");

    let opened = sim.talk(1, "keeper");

    assert_eq!(lines(&opened), vec!["Welcome in.".to_owned()]);
    assert_eq!(opened.with, Some(keeper));
    // A step under way when the greeting opens still ends on its tile.
    assert!(sim.run_until(1.0, |world| !moving(world, player)));
    let at = position(&sim.world, player).expect("position");
    let keeper_at = position(&sim.world, keeper).expect("position");
    assert!(at.distance(keeper_at) <= Meters(2.0 + std::f32::consts::SQRT_2));
    assert!(commands_locked(&sim.world, player));
    assert_eq!(action(&sim.world, player), Action::Talk);
    assert_eq!(action(&sim.world, keeper), Action::Talk);
    assert!(
        faces(&sim.world, player, keeper_at) && faces(&sim.world, keeper, at),
        "{:?} and {:?} talk without facing each other",
        facing(&sim.world, player),
        facing(&sim.world, keeper)
    );

    let conn = sim.conn(1);
    sim.send(
        conn,
        MoveRequest {
            pos: at + Offset::new(0.0, 4.0),
            seq: MoveSeq(1),
        },
    );
    sim.run(2.0);
    let still = position(&sim.world, player).expect("position");
    assert!(still.distance(at) < Meters(0.5));

    sim.send(conn, ConversationRequest::Leave { step: opened.step });
    sim.tick();
    assert!(sim.conversation(1).is_none());
    assert!(!commands_locked(&sim.world, player));
    assert_eq!(action(&sim.world, player), Action::Idle);
    assert_eq!(action(&sim.world, keeper), Action::Idle);
}

fn action(world: &World, actor: Entity) -> Action {
    world.get::<Actor>(actor).expect("an actor").action
}

fn facing(world: &World, actor: Entity) -> Radians {
    world.get::<Actor>(actor).expect("an actor").facing
}

/// Whether the actor faces `there` to within a hair.
fn faces(world: &World, actor: Entity, there: Pos<Meters>) -> bool {
    let at = position(world, actor).expect("position");
    let wanted = Radians::of(there - at);
    let turned = facing(world, actor);
    (turned.0 - wanted.0).sin().abs() < 1e-3 && (turned.0 - wanted.0).cos() > 0.0
}

#[test]
fn a_pick_answers_its_step_once() {
    let mut sim = inn();
    let player = sim.join(1);
    sim.give(player, "gold", 30);
    let opened = sim.talk(1, "keeper");
    let round = ConversationRequest::Pick {
        step: opened.step,
        choice: choice(&opened, "Buy a round for the room."),
    };

    let conn = sim.conn(1);
    sim.send(conn, round);
    sim.send(conn, round);
    sim.tick();

    assert_eq!(sim.count(player, "gold"), 20);
    let now = sim.conversation(1).expect("still talking");
    assert_eq!(lines(&now), vec!["Cheers!".to_owned()]);
    assert_eq!(favour(&sim, player), Recalled::Int(1));

    sim.send(conn, round);
    sim.tick();
    assert_eq!(sim.count(player, "gold"), 20);
}

#[test]
fn a_choice_that_costs_too_much_changes_nothing() {
    let mut sim = inn();
    let player = sim.join(1);
    let opened = sim.talk(1, "keeper");
    let index = choice(&opened, "Buy a round for the room.");
    assert!(!opened.choices[index as usize].locked);

    let conn = sim.conn(1);
    sim.send(
        conn,
        ConversationRequest::Pick {
            step: opened.step,
            choice: index,
        },
    );
    sim.tick();

    let now = sim.conversation(1).expect("still talking");
    assert_eq!(lines(&now), vec!["Welcome in.".to_owned()]);
    assert_eq!(now.refused.map(|refused| refused.choice), Some(index));
    assert_eq!(sim.errors(1), vec!["Needs 10 more Gold".to_owned()]);
    assert_eq!(favour(&sim, player), Recalled::Int(0));
}

#[test]
fn walking_out_of_reach_or_losing_the_speaker_ends_the_conversation() {
    let mut sim = inn();
    let player = sim.join(1);
    sim.talk(1, "keeper");
    let at = position(&sim.world, player).expect("position");

    movement::relocate(&mut sim.world, player, at + Offset::new(0.0, 7.0));
    sim.tick();

    assert!(sim.conversation(1).is_none());
    assert!(!commands_locked(&sim.world, player));

    sim.talk(1, "keeper");
    let keeper = sim.npc("keeper");
    npc::despawn(&mut sim.world, keeper);
    sim.tick();

    assert!(sim.conversation(1).is_none());
    assert!(!commands_locked(&sim.world, player));
}

#[test]
fn damage_keeps_a_conversation_but_death_ends_it() {
    let mut sim = inn();
    let player = sim.join(1);
    sim.talk(1, "keeper");

    stat::apply_damage(&mut sim.world, player, 1.0);
    sim.tick();
    assert!(sim.conversation(1).is_some());

    sim.world.entity_mut(player).remove::<Immortal>();
    let health = stat::effective(&sim.world, player, StatKind::Health);
    stat::apply_damage(&mut sim.world, player, health);
    sim.tick();
    assert!(sim.conversation(1).is_none());
}

#[test]
fn a_conversation_an_npc_starts_replaces_the_one_in_progress() {
    let mut sim = inn();
    let player = sim.join(1);
    let keeper = sim.npc("keeper");
    sim.talk(1, "keeper");

    let crier = sim.npc("crier");
    let owner = script::owner_of(&sim.world, crier).expect("the crier runs scripts");
    sim.world.resource_mut::<RiftInstances>().raise(
        owner,
        EventKind::Interacted,
        Some(player),
        Vec::new(),
        |_| true,
    );
    sim.tick();

    let opened = sim.conversation(1).expect("talking");
    assert_eq!(opened.with, Some(crier));
    assert_ne!(opened.with, Some(keeper));
    assert_eq!(lines(&opened), vec!["The crier tips his hat.".to_owned()]);
}

#[test]
fn news_called_out_waits_until_you_are_free() {
    let mut sim = inn();
    let player = sim.join(1);
    let opened = sim.talk(1, "crier");
    assert_eq!(lines(&opened), vec!["The crier tips his hat.".to_owned()]);

    sim.run(1.0);
    assert_eq!(sim.conversation(1).map(|now| now.step), Some(opened.step));
    let conn = sim.conn(1);
    sim.send(conn, ConversationRequest::Leave { step: opened.step });
    assert!(sim.run_until(1.0, |world| {
        world
            .get::<Conversation>(player)
            .is_some_and(|now| now.step != opened.step)
    }));
    let news = sim.conversation(1).expect("the crier's news");
    assert_eq!(lines(&news), vec!["Hear ye!".to_owned()]);

    sim.send(conn, ConversationRequest::Leave { step: news.step });
    sim.run(2.0);
    assert!(sim.conversation(1).is_none());
}

#[test]
fn an_npc_shows_news_only_to_those_who_have_not_heard_him_out() {
    let mut sim = inn();
    let regular = sim.join(1);
    let stranger = sim.join(2);
    let crier = sim.npc("crier");
    let step = sim.talk(1, "crier").step;
    let conn = sim.conn(1);
    sim.send(conn, ConversationRequest::Leave { step });
    sim.tick();
    sim.world.insert_resource(WallClock {
        now: UnixMillis(3_600_000),
        reset: UtcHour::MIDNIGHT,
    });

    sim.run(0.5);

    let marks = |sim: &Sim, player: Entity| -> Vec<AttentionId> {
        sim.world
            .get::<Attention>(player)
            .expect("attention")
            .of(crier)
            .to_vec()
    };
    let news = AttentionMark::News.id(sim.world.resource());
    assert!(!marks(&sim, regular).contains(&news));
    assert!(marks(&sim, stranger).contains(&news));
}

fn inn() -> Sim {
    let content = Game::new()
        .area(
            "inn",
            &Map::open(15, 15),
            json!([
                { "type": "resident", "npc": "keeper.npc.json", "at": { "x": 7.0, "y": 4.0 } },
                { "type": "resident", "npc": "crier.npc.json", "at": { "x": 7.0, "y": 13.0 } },
            ]),
        )
        .hero("inn", 7.0, 7.0)
        .item(
            "gold",
            json!({
                "name": "Gold",
                "kind": { "type": "resource" },
                "modules": [{ "type": "stack", "max": 100_000 }],
            }),
        )
        .npc(
            "keeper",
            40.0,
            json!([{ "type": "script", "script": "keeper_talk.script.json" }]),
        )
        .script("keeper_talk", "npc", keeping())
        .busts("keeper")
        .npc(
            "crier",
            40.0,
            json!([{ "type": "script", "script": "crier_talk.script.json" }]),
        )
        .script("crier_talk", "npc", crying())
        .row(
            "memory",
            "favour",
            json!({ "label": "Rounds bought", "type": "integer", "resets": "never" }),
        )
        .memory("met", "never")
        .memory("news_today", "daily")
        .mark("news")
        .mark("talking")
        .sfx("ui_refuse")
        .sfx("ui_close")
        .vfx("damage_taken")
        .shader("damage_pop")
        .vfx("defeated")
        .load();
    let mut sim = Sim::new(content);
    sim.serve("inn")
        .accept::<MoveRequest>()
        .accept::<InteractRequest>()
        .accept::<ConversationRequest>()
        .add((
            movement::move_request,
            interact::interact_request,
            dialogue::requests,
        ))
        .add((
            script::query::take_tick_roster,
            npc::approach,
            script::run_tick,
            npc::body::drive,
            movement::advance,
            interact::interactions,
            dialogue::hold,
            dialogue::refresh,
        ))
        .add((
            stat::settle_health,
            combat::defeats,
            actor::settle,
            attention::update,
        ));
    script::spawn_stateless(&mut sim.world);
    npc::spawn_all(&mut sim.world);
    sim
}

fn keeping() -> Value {
    json!({
            "interacted": {
                "type": "on",
                "event": "interacted",
                "verb": "talk",
                "reach": 2.0,
                "next": "hello",
            },
            "hello": {
                "type": "dialogue",
                "lines": [{
                    "by": { "type": "npc", "npc": "keeper.npc.json", "face": "neutral" },
                    "text": ["Welcome in."],
                }],
                "choices": [
                    { "label": ["Buy a round for the room."], "reveal": ["costs"], "to": "pay_round" },
                    { "label": ["Goodbye."] },
                ],
            },
            "pay_round": {
                "type": "items",
                "take": [{ "item": "gold.item.json", "count": 10 }],
                "done": "count_round",
            },
            "count_round": {
                "type": "remember",
                "memory": "favour.memory.json",
                "to": { "type": "add", "a": { "type": "memory", "memory": "favour.memory.json" }, "b": 1 },
                "next": "round",
            },
            "round": {
                "type": "dialogue",
                "lines": [{ "by": { "type": "npc", "npc": "keeper.npc.json", "face": "happy" }, "text": ["Cheers!"] }],
                "choices": [{ "label": ["Goodbye."] }],
            },
    })
}

fn crying() -> Value {
    json!({
            "interacted": {
                "type": "on",
                "event": "interacted",
                "verb": "talk",
                "reach": 2.0,
                "news": true,
                "next": "meet",
            },
            "meet": { "type": "remember", "memory": "met.memory.json", "next": "hello" },
            "hello": {
                "type": "dialogue",
                "lines": [{ "by": { "type": "narrator" }, "text": ["The crier tips his hat."] }],
                "choices": [{ "label": ["Goodbye."] }],
            },
            "came_near": {
                "type": "on",
                "event": "came_near",
                "within": 3.0,
                "by": "characters",
                "next": "news_due",
            },
            "news_due": {
                "type": "if",
                "test": {
                    "type": "all",
                    "of": [
                        { "type": "memory", "memory": "met.memory.json" },
                        { "type": "not", "of": { "type": "memory", "memory": "news_today.memory.json" } },
                    ],
                },
                "yes": "remember_news",
            },
            "remember_news": { "type": "remember", "memory": "news_today.memory.json", "next": "news" },
            "news": {
                "type": "dialogue",
                "lines": [{ "by": { "type": "narrator" }, "text": ["Hear ye!"] }],
                "choices": [{ "label": ["Thanks."] }],
            },
    })
}

fn favour(sim: &Sim, player: Entity) -> Recalled {
    let favour = sim.id::<MemoryDef>("favour");
    memory::get(&sim.world, player, favour)
}

fn choice(conversation: &Conversation, label: &str) -> u32 {
    conversation
        .choices
        .iter()
        .position(|choice| choice.label.plain_words() == label)
        .expect("the choice") as u32
}
