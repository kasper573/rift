use bevy::prelude::{App, MinimalPlugins};
use bevy_ecs::prelude::*;
use engine::core::content::Content;
use engine::core::content::Fixture;
use engine::core::math::{Offset, Pos};
use engine::core::script::DialogueCamera;
use engine::core::script::EventKind;
use engine::core::script::Instances;
use engine::core::script::Value as Recalled;
use engine::core::time::{UnixMillis, UtcHour, WallClock};
use engine::core::units::{MeterPos, Meters, Radians};
use engine::systems::actor::render::RenderActor;
use engine::systems::actor::{self, Action, Actor};
use engine::systems::attention::{self, Attention, AttentionId, AttentionMark};
use engine::systems::dialogue::{self, Conversation, ConversationRequest};
use engine::systems::identity::Names;
use engine::systems::interact::Interacted;
use engine::systems::interact::{self, InteractRequest};
use engine::systems::memory::{self, MemoryDef};
use engine::systems::movement::{self, MoveRequest, MoveSeq, Position, moving, position};
use engine::systems::player::commands_locked;
use engine::systems::player::session::{ClientSessionPlugin, MyCharacter, MyClient};
use engine::systems::player::{ClientId, PlayedBy};
use engine::systems::probe;
use engine::systems::region;
use engine::systems::stat::{self, Immortal, StatKind};
use engine::systems::{combat, npc};
use serde_json::{Value, json};
use testkit::{BODY, Game, Map, Sim, sent};

#[test]
fn a_client_with_no_window_sees_its_conversation_and_answers_it() {
    let mut sim = inn();
    let player = sim.join(1);
    sim.give(player, "gold", 30);
    let opened = sim.talk(1, "keeper");

    let mut client = App::new();
    client
        .add_plugins((MinimalPlugins, ClientSessionPlugin))
        .insert_resource(sim.world.resource::<Content>().clone())
        .insert_resource(MyClient(Some(MINE)));
    client.world_mut().spawn((
        Names::for_world(0).give(),
        PlayedBy { client: MINE },
        sent(&opened),
    ));
    client.update();
    let seen = probe::sight(client.world_mut())
        .conversation
        .expect("the client sees the conversation its character is in");
    let said: Vec<String> = seen.lines.iter().map(|line| line.text.clone()).collect();
    let offered: Vec<String> = seen
        .choices
        .iter()
        .map(|choice| choice.label.clone())
        .collect();
    assert_eq!(said, sim.lines(&opened));
    assert_eq!(offered, sim.labels(&opened));

    let round = offered
        .iter()
        .position(|label| label == "Buy a round for the room.")
        .expect("the round is offered");
    let conn = sim.conn(1);
    sim.send(
        conn,
        ConversationRequest::Pick {
            step: seen.step,
            choice: round as u32,
        },
    );
    sim.run(0.5);
    let answered = sim.conversation(1).expect("still talking");
    assert_ne!(
        answered.step, opened.step,
        "the answer the client saw how to give was taken"
    );
}

#[test]
fn a_conversation_is_seen_as_its_script_says_but_for_a_line_that_says_otherwise() {
    let mut sim = inn();
    sim.join(1);

    let greeting = sim.talk(1, "keeper");
    assert_eq!(
        (greeting.camera, greeting.lines[0].camera),
        (DialogueCamera::Participants, Some(DialogueCamera::Gameplay)),
        "the keeper's script names a camera, and its greeting another"
    );
    assert!(
        greeting.lines[0].gesture.is_some(),
        "the keeper greets with the nod its line names"
    );
    let conn = sim.conn(1);
    sim.send(
        conn,
        ConversationRequest::Leave {
            step: greeting.step,
        },
    );
    sim.tick();

    let tipped = sim.talk(1, "crier");
    assert_eq!(
        (tipped.camera, tipped.lines[0].camera),
        (DialogueCamera::Shoulder, None),
        "a script that names none is seen over the shoulder"
    );
}

#[test]
fn whoever_else_says_a_line_turns_to_the_player_while_it_shows() {
    let mut sim = inn();
    let player = sim.join(1);
    let crier = sim.npc("crier");
    sim.give(player, "gold", 30);
    let opened = sim.talk(1, "keeper");
    assert_eq!(action(&sim.world, crier), Action::Idle);

    let conn = sim.conn(1);
    sim.send(
        conn,
        ConversationRequest::Pick {
            step: opened.step,
            choice: choice(&sim, &opened, "Buy a round for the room."),
        },
    );
    sim.run(0.5);
    let round = sim.conversation(1).expect("still talking");
    let at = position(&sim.world, player).expect("position");
    assert_eq!(action(&sim.world, crier), Action::Talk);
    assert!(
        faces(&sim.world, crier, at),
        "the crier cheers facing {:?}",
        facing(&sim.world, crier)
    );

    sim.send(conn, ConversationRequest::Leave { step: round.step });
    sim.run(0.5);
    assert_eq!(action(&sim.world, crier), Action::Idle);
}

#[test]
fn talking_walks_into_reach_opens_the_greeting_and_locks_commands() {
    let mut sim = inn();
    let player = sim.join(1);
    let keeper = sim.npc("keeper");
    sim.world.insert_resource(MyCharacter(Some(player)));
    movement::walk_ahead(&mut sim.world);

    let opened = sim.talk(1, "keeper");

    assert_eq!(sim.lines(&opened), vec!["Welcome in.".to_owned()]);
    assert_eq!(opened.with, Some(keeper));
    // A step under way when the greeting opens still ends on its tile, and the stance settles
    // the tick after.
    assert!(sim.run_until(1.0, |world| !moving(world, player)));
    sim.tick();
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
    let seen = (0..200)
        .find_map(|_| {
            sim.tick();
            movement::walk_ahead(&mut sim.world);
            let seen = sim.world.get::<RenderActor>(player).copied()?;
            (!seen.action.moves()).then_some(seen)
        })
        .expect("the player's own character comes to rest on their screen");
    assert_eq!(
        (seen.facing, seen.action),
        (facing(&sim.world, player), Action::Talk),
        "the player sees their own character turned to the keeper as everyone else does"
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
fn a_pick_is_answered_once_and_a_choice_that_costs_too_much_changes_nothing() {
    let mut sim = inn();
    let player = sim.join(1);
    let opened = sim.talk(1, "keeper");
    let index = choice(&sim, &opened, "Buy a round for the room.");
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

    let refused = sim.conversation(1).expect("still talking");
    assert_eq!(sim.lines(&refused), vec!["Welcome in.".to_owned()]);
    assert_eq!(refused.refused.map(|refused| refused.choice), Some(index));
    assert_eq!(sim.errors(1), vec!["Needs 10 Gold more".to_owned()]);
    assert_eq!(favour(&sim, player), Recalled::Int(0));

    sim.give(player, "gold", 30);
    let round = ConversationRequest::Pick {
        step: refused.step,
        choice: index,
    };
    sim.send(conn, round);
    sim.send(conn, round);
    sim.tick();

    assert_eq!(sim.count(player, "gold"), 20);
    let now = sim.conversation(1).expect("still talking");
    assert_eq!(
        sim.lines(&now),
        vec!["Cheers!".to_owned(), "Hear, hear!".to_owned()]
    );
    assert_eq!(favour(&sim, player), Recalled::Int(1));

    sim.send(conn, round);
    sim.tick();
    assert_eq!(sim.count(player, "gold"), 20);
}

#[test]
fn walking_out_of_reach_losing_the_speaker_a_blow_or_death_ends_the_conversation_but_damage_does_not()
 {
    let mut sim = inn();
    let player = sim.join(1);
    let keeper = sim.npc("keeper");
    let ended = |sim: &mut Sim, how: &str| {
        assert!(sim.conversation(1).is_none(), "{how} ends the talk");
        assert!(!commands_locked(&sim.world, player), "{how}");
        sim.tick();
        assert_eq!(action(&sim.world, player), Action::Idle, "{how}");
    };
    sim.talk(1, "keeper");
    let at = position(&sim.world, player).expect("position");

    movement::relocate(&mut sim.world, player, at + Offset::new(0.0, 7.0));
    sim.tick();
    ended(&mut sim, "walking out of reach");

    sim.talk(1, "keeper");
    stat::apply_damage(&mut sim.world, player, 1.0);
    sim.tick();
    assert!(sim.conversation(1).is_some(), "damage from no one keeps it");

    let crier = sim.npc("crier");
    combat::inflict(&mut sim.world, player, Some(crier), 1.0);
    sim.tick();
    ended(&mut sim, "a blow");
    assert_eq!(action(&sim.world, keeper), Action::Idle);

    sim.talk(1, "crier");
    npc::despawn(&mut sim.world, crier);
    sim.tick();
    ended(&mut sim, "losing the speaker");

    sim.talk(1, "keeper");
    sim.world.entity_mut(player).remove::<Immortal>();
    let health = stat::effective(&sim.world, player, StatKind::Health);
    stat::apply_damage(&mut sim.world, player, health);
    sim.tick();
    assert!(sim.conversation(1).is_none(), "death ends the talk");
}

#[test]
fn a_conversation_an_npc_starts_replaces_the_one_in_progress() {
    let mut sim = inn();
    let player = sim.join(1);
    let keeper = sim.npc("keeper");
    sim.talk(1, "keeper");

    let crier = sim.npc("crier");
    let owner =
        engine::systems::area::script::owner_of(&sim.world, crier).expect("the crier runs scripts");
    sim.world.resource_mut::<Instances>().raise(
        owner,
        EventKind::of::<Interacted>(),
        Some(player),
        Vec::new(),
        |_| true,
    );
    sim.tick();

    let opened = sim.conversation(1).expect("talking");
    assert_eq!(opened.with, Some(crier));
    assert_ne!(opened.with, Some(keeper));
    assert_eq!(
        sim.lines(&opened),
        vec!["The crier tips his hat.".to_owned()]
    );
}

/// Ending a conversation is no coming near: whoever talked with an npc can walk off when it ends,
/// however close they stand. News called out to someone who comes near while they talk waits
/// until they are free.
#[test]
fn news_called_out_waits_until_you_are_free_and_a_talk_ending_calls_out_nothing() {
    let mut sim = inn();
    let player = sim.join(1);
    let conn = sim.conn(1);
    let met = sim.talk(1, "crier");
    assert_eq!(sim.lines(&met), vec!["The crier tips his hat.".to_owned()]);
    sim.run(1.0);
    sim.send(conn, ConversationRequest::Leave { step: met.step });
    sim.run(2.0);
    assert!(
        sim.conversation(1).is_none(),
        "leaving the crier's talk, close by him, is no coming near him"
    );

    let kept = sim.talk(1, "keeper");
    let crier = sim.npc("crier");
    let beside = position(&sim.world, player).expect("the player stands") + Offset::new(1.0, 0.0);
    sim.world.entity_mut(crier).insert(Position::at(beside));
    sim.run(1.0);
    assert_eq!(
        sim.conversation(1).map(|now| now.step),
        Some(kept.step),
        "the crier's news waits while the keeper talks"
    );
    sim.send(conn, ConversationRequest::Leave { step: kept.step });
    assert!(sim.run_until(1.0, |world| {
        world
            .get::<Conversation>(player)
            .is_some_and(|now| now.step != kept.step)
    }));
    let news = sim.conversation(1).expect("the crier's news");
    assert_eq!(sim.lines(&news), vec!["Hear ye!".to_owned()]);
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

    let teller = sim.npc("teller");
    assert!(
        sim.world
            .get::<Attention>(stranger)
            .expect("attention")
            .of(teller)
            .contains(&news),
        "a tale behind a random pick is news until it is heard"
    );
}

fn inn() -> Sim {
    let content = Game::new()
        .area(
            "inn",
            &Map::open(15, 15),
            json!([
                { "type": "resident", "npc": "keeper.npc.json", "at": { "x": 7.0, "y": 4.0 } },
                { "type": "resident", "npc": "crier.npc.json", "at": { "x": 7.0, "y": 13.0 } },
                { "type": "resident", "npc": "teller.npc.json", "at": { "x": 1.0, "y": 13.0 } },
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
        .edit("script", "keeper_talk", |script| {
            script["camera"] = json!("participants")
        })
        .edit("skeleton", BODY, |body| {
            body["base"]["gestures"] = json!({ "nod": testkit::clip("attack") })
        })
        .npc(
            "crier",
            40.0,
            json!([{ "type": "script", "script": "crier_talk.script.json" }]),
        )
        .script("crier_talk", "npc", crying())
        .npc(
            "teller",
            40.0,
            json!([{ "type": "script", "script": "teller_talk.script.json" }]),
        )
        .script(
            "teller_talk",
            "npc",
            json!({
                "interacted": {
                    "type": "on",
                    "event": "interacted",
                    "verb": "talk",
                    "reach": 2.0,
                    "news": true,
                    "next": "roll",
                },
                "roll": { "type": "random", "weights": [1, 1], "outputs": ["tale", "shrug"] },
                "tale": {
                    "type": "dialogue",
                    "lines": [{ "by": { "type": "narrator" }, "text": ["Once upon a time."] }],
                    "choices": [{ "label": ["Thanks."] }],
                },
                "shrug": { "type": "wait", "seconds": 1, "at_most": 5.0 },
            }),
        )
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
        .add(movement::file_by_chunk)
        .plan((
            engine::systems::roster::ListRoster::default(),
            npc::ApproachWatchers::default(),
            engine::core::script::run::FlowRunner::default(),
            npc::body::DriveBodies::default(),
            movement::AdvanceWalks::default(),
        ))
        .add((interact::interactions, dialogue::hold, dialogue::refresh))
        .plan(stat::SettleVitals::default())
        .add(combat::defeats)
        .plan((
            actor::SettleActions::default(),
            attention::MarkAttention::default(),
        ));
    engine::systems::spawn_scripts(&mut sim.world);
    region::hold_whole(&mut sim.world);
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
                    "by": { "type": "npc", "npc": "keeper.npc.json", "gesture": "nod" },
                    "camera": "gameplay",
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
                "lines": [
                    { "by": { "type": "npc", "npc": "keeper.npc.json" }, "text": ["Cheers!"] },
                    { "by": { "type": "npc", "npc": "crier.npc.json" }, "text": ["Hear, hear!"] },
                ],
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
                "fliers": false,
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

fn choice(sim: &Sim, conversation: &Conversation, label: &str) -> u32 {
    conversation
        .choices
        .iter()
        .position(|choice| choice.label.plain_words(sim.content()) == label)
        .expect("the choice") as u32
}

const MINE: ClientId = ClientId(1);
