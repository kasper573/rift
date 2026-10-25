use engine::core::time::{UnixMillis, UtcHour, WallClock};
use engine::systems::dialogue::{self, Conversation, ConversationRequest};
use engine::systems::interact::{self, InteractRequest};
use engine::systems::player::commands_locked;
use engine::systems::{area, movement, script};
use serde_json::{Value, json};
use testkit::{Game, Map, Sim, dialogue, lines, path};

const DAY: u64 = 24 * 3_600_000;

#[test]
fn reading_a_map_object_opens_its_conversation_with_the_object() {
    let mut sim = beach();
    let player = sim.join(1);
    let board = sim.object("board");

    let opened = sim.open_with(1, board);

    assert_eq!(lines(&opened), vec!["Notices flap on the post.".to_owned()]);
    assert_eq!(opened.with, Some(board));
    assert!(commands_locked(&sim.world, player));
}

#[test]
fn a_chest_gives_its_gold_once_a_day() {
    let mut sim = beach();
    let player = sim.join(1);
    let chest = sim.object("chest");

    let found = sim.open_with(1, chest);
    assert_eq!(
        lines(&found),
        vec!["Five coins glint in the sand.".to_owned()]
    );
    assert_eq!(sim.count(player, "gold"), 5);
    sim.leave(1);

    let empty = sim.open_with(1, chest);
    assert_eq!(lines(&empty), vec!["Only sand.".to_owned()]);
    assert_eq!(sim.count(player, "gold"), 5);
    sim.leave(1);

    sim.world.insert_resource(WallClock {
        now: UnixMillis(DAY),
        reset: UtcHour::MIDNIGHT,
    });
    let refilled = sim.open_with(1, chest);
    assert_eq!(
        lines(&refilled),
        vec!["Five coins glint in the sand.".to_owned()]
    );
    assert_eq!(sim.count(player, "gold"), 10);
}

#[test]
fn two_players_open_the_same_chest_for_themselves() {
    let mut sim = beach();
    let first = sim.join(1);
    let second = sim.join(2);
    let chest = sim.object("chest");

    for client in [1, 2] {
        let conn = sim.conn(client);
        sim.send(conn, InteractRequest { target: chest });
    }
    assert!(sim.run_until(20.0, |world| {
        world.get::<Conversation>(first).is_some() && world.get::<Conversation>(second).is_some()
    }));

    for (client, player) in [(1, first), (2, second)] {
        let opened = sim.conversation(client).expect("open");
        assert_eq!(
            lines(&opened),
            vec!["Five coins glint in the sand.".to_owned()]
        );
        assert_eq!(sim.count(player, "gold"), 5);
    }
}

fn beach() -> Sim {
    let content = Game::new()
        .area(
            "beach",
            &Map::open(9, 9),
            json!([
                object("board", 2.0, 4.0, "board_read"),
                object("chest", 6.0, 4.0, "chest_open"),
            ]),
        )
        .script(
            "board_read",
            "object",
            json!({
                "interacted": { "type": "on", "event": "interacted", "verb": "read", "reach": 1.5, "next": "read" },
                "read": said("Notices flap on the post."),
            }),
        )
        .script(
            "chest_open",
            "object",
            json!({
                "interacted": { "type": "on", "event": "interacted", "verb": "open", "reach": 1.5, "next": "looted" },
                "looted": {
                    "type": "if",
                    "test": { "type": "memory", "memory": "chest_looted.memory.json" },
                    "yes": "empty",
                    "no": "loot",
                },
                "loot": {
                    "type": "items",
                    "give": [{ "item": "gold.item.json", "count": 5 }],
                    "done": "remember",
                },
                "remember": { "type": "remember", "memory": "chest_looted.memory.json", "next": "found" },
                "found": said("Five coins glint in the sand."),
                "empty": said("Only sand."),
            }),
        )
        .hero("beach", 4.0, 4.0)
        .item(
            "gold",
            json!({ "kind": { "type": "resource" }, "modules": [{ "type": "stack", "max": 1000 }] }),
        )
        .memory("chest_looted", "daily")
        .prop("look", json!({ "type": "none" }))
        .load();
    let mut sim = Sim::new(content);
    sim.serve("beach")
        .accept::<InteractRequest>()
        .accept::<ConversationRequest>()
        .add((interact::interact_request, dialogue::requests))
        .add((
            script::run_tick,
            movement::advance,
            interact::interactions,
            dialogue::hold,
            dialogue::refresh,
        ));
    script::spawn_stateless(&mut sim.world);
    area::object::spawn_all(&mut sim.world);
    sim
}

fn object(name: &str, x: f32, y: f32, script: &str) -> Value {
    json!({
        "type": "object",
        "name": name,
        "title": name,
        "shape": { "type": "point", "x": x, "y": y },
        "look": path("prop", "look"),
        "scripts": [{ "script": path("script", script) }],
    })
}

fn said(text: &str) -> Value {
    dialogue(text, json!([{ "label": ["Step back."] }]))
}
