use bevy_ecs::message::MessageCursor;
use bevy_replication::ToClient;
use bevy_terminal::{
    LineTone, TerminalCompletionRequest, TerminalCompletions, TerminalInput, TerminalLine,
};
use engine::core::content::Content;
use engine::core::net::auth::TokenClaims;
use engine::systems::account::{AccountIdentity, RoleDef};
use engine::systems::{chat, command, movement, script};
use serde_json::{Value, json};
use testkit::{Game, Map, Sim, path};

#[test]
fn a_command_runs_its_script_on_what_was_typed_and_says_why_it_cannot() {
    let mut sim = camp(game().load());
    let (me, friend) = (sim.join(1), sim.join(2));

    send(&mut sim, 1, "/give potion");
    send(&mut sim, 1, "/give Potion, 3, player 2");
    sim.tick();
    assert_eq!(
        (sim.count(me, "potion"), sim.count(friend, "potion")),
        (1, 3),
        "an input left out takes its default: one potion, to whoever sent the command"
    );

    for (line, refusal) in [
        ("/dance", "There is no command dance"),
        ("/give", "/give <item>, [amount], [player] needs its item"),
        ("/give rock", "rock is no item"),
        ("/give potion, many", "many is no amount"),
        ("/give potion, 1, nobody", "nobody is no player"),
        (
            "/give potion, 1, player 2, more",
            "/give <item>, [amount], [player] takes no more than that",
        ),
    ] {
        send(&mut sim, 1, line);
        assert_eq!(
            lines(&mut sim, 1).last(),
            Some(&(refusal.to_owned(), LineTone::Failed)),
            "{line}"
        );
    }
    assert_eq!(sim.count(me, "potion"), 1, "a refused command does nothing");

    send(&mut sim, 1, "/warp camp, 3, 4");
    sim.tick();
    let at = movement::position(&sim.world, me).expect("somewhere");
    assert_eq!(
        (at.x, at.y),
        (3.0, 4.0),
        "a place is an area, then where in it"
    );

    send(&mut sim, 2, "/commands");
    sim.tick();
    let told = lines(&mut sim, 2);
    assert_eq!(
        told[told.len() - 3..],
        [
            "/commands",
            "/give <item>, [amount], [player]",
            "/warp <to>, [x], [y]"
        ]
        .map(|usage| (usage.to_owned(), LineTone::Said))
    );
}

#[test]
fn a_command_is_for_the_roles_it_names_which_an_account_holds_by_what_its_token_claims() {
    let content = game()
        .row(
            "role",
            "keeper",
            json!({ "name": "Keeper", "claim": "/realm/roles", "value": "keeper" }),
        )
        .edit("command", "warp", |warp| {
            warp["roles"] = json!([path("role", "keeper")]);
        })
        .load();
    let keeper = content
        .by_path::<RoleDef>(&path("role", "keeper"))
        .expect("the keeper role");
    for (claims, held) in [
        (json!({ "realm": { "roles": ["player", "keeper"] } }), true),
        (json!({ "realm": { "roles": "player keeper" } }), true),
        (json!({ "realm": { "roles": ["player"] } }), false),
        (json!({ "roles": ["keeper"] }), false),
        (json!({}), false),
    ] {
        assert_eq!(
            account(&content, &claims).roles.contains(&keeper),
            held,
            "{claims}"
        );
    }

    let mut sim = camp(content.clone());
    sim.join(1);
    sim.join(2);
    let (keeper, player) = (sim.conn(1), sim.conn(2));
    let claims = json!({ "realm": { "roles": ["keeper"] } });
    sim.world
        .entity_mut(keeper)
        .insert(account(&content, &claims));
    sim.world
        .entity_mut(player)
        .insert(account(&content, &json!({})));

    assert_eq!(
        (offered(&mut sim, 1, "/"), offered(&mut sim, 2, "/")),
        (
            vec![
                "commands".to_owned(),
                "give ".to_owned(),
                "warp ".to_owned()
            ],
            vec!["commands".to_owned(), "give ".to_owned()]
        ),
        "a command is only offered to those who may send it"
    );
    send(&mut sim, 2, "/warp camp");
    assert_eq!(
        lines(&mut sim, 2).last(),
        Some(&("There is no command warp".to_owned(), LineTone::Failed)),
        "to anyone else the command is no command at all"
    );
    send(&mut sim, 1, "/warp camp, 1, 1");
    assert!(lines(&mut sim, 1).is_empty(), "the keeper's warp is taken");
}

#[test]
fn a_command_being_typed_goes_on_by_name_and_then_by_what_the_input_at_hand_takes() {
    let content = game()
        .item("plank", json!({ "kind": { "type": "resource" } }))
        .item("pickaxe", json!({ "kind": { "type": "resource" } }))
        .edit_settings(|settings| settings["suggestions"] = json!(2))
        .load();
    let mut sim = camp(content);
    let (me, near, far) = (sim.join(1), sim.join(2), sim.join(3));
    for (who, x) in [(me, 1.0), (near, 2.0), (far, 4.0)] {
        movement::relocate(&mut sim.world, who, engine::core::math::Pos::new(x, 1.0));
    }

    assert_eq!(
        offered(&mut sim, 1, "hello"),
        [""; 0],
        "only commands go on"
    );
    assert_eq!(
        offered(&mut sim, 1, "/"),
        ["commands", "give "],
        "no more than the game's settings offer at once"
    );
    assert_eq!(
        suggested(&mut sim, 1, "/g"),
        TerminalCompletions {
            text: "/g".to_owned(),
            keeps: 1,
            options: vec![option("give ", "/give <item>, [amount], [player]")],
            usage: String::new(),
        },
        "a name goes on to the commands it may become, each spelled out"
    );
    assert_eq!(
        suggested(&mut sim, 1, "/give p"),
        TerminalCompletions {
            text: "/give p".to_owned(),
            keeps: 6,
            options: vec![option("pickaxe", "item"), option("plank", "item")],
            usage: "item: /give <item>, [amount], [player]".to_owned(),
        },
        "once a command is named, its first input goes on to what the input's type takes"
    );
    assert_eq!(
        suggested(&mut sim, 1, "/give plank, "),
        TerminalCompletions {
            text: "/give plank, ".to_owned(),
            keeps: 13,
            options: Vec::new(),
            usage: "amount: /give <item>, [amount], [player]".to_owned(),
        },
        "an input that takes anything of its type only says what it is"
    );
    assert_eq!(
        suggested(&mut sim, 1, "/give plank, 2, ").options,
        [option("player 1", "0 m"), option("player 2", "1 m")],
        "a player goes on to those playing nearest"
    );
    assert_eq!(
        offered(&mut sim, 1, "/give plank, 2, 3"),
        ["player 3"],
        "and to whoever is called as typed, however far"
    );
    assert_eq!(offered(&mut sim, 1, "/warp ca"), ["camp"]);
    assert_eq!(
        suggested(&mut sim, 1, "/warp camp, 3, ").usage,
        "y: /warp <to>, [x], [y]"
    );
}

fn option(text: &str, detail: &str) -> bevy_terminal::TerminalOption {
    bevy_terminal::TerminalOption {
        text: text.to_owned(),
        detail: detail.to_owned(),
    }
}

/// A camp with a potion, and three commands: one giving an item, one warping to a place, one
/// listing the commands.
fn game() -> Game {
    Game::new()
        .area("camp", &Map::open(6, 6), json!([]))
        .hero("camp", 2.0, 2.0)
        .item("potion", json!({ "kind": { "type": "resource" } }))
        .row(
            "script",
            "give",
            json!({
                "runs_on": "command",
                "variables": {
                    "item": { "type": "row", "table": "item" },
                    "amount": { "type": "integer", "default": 1 },
                    "player": { "type": "character", "default": { "type": "subject" } },
                },
                "nodes": {
                    "sent": { "type": "on", "event": "invoked", "next": "hand" },
                    "hand": {
                        "type": "items",
                        "who": { "var": "player" },
                        "give": [{ "item": { "var": "item" }, "count": { "var": "amount" } }],
                    },
                },
            }),
        )
        .row(
            "command",
            "give",
            json!({ "script": path("script", "give"), "inputs": ["item", "amount", "player"] }),
        )
        .row(
            "script",
            "warp",
            json!({
                "runs_on": "command",
                "variables": { "to": { "type": "place" } },
                "nodes": {
                    "sent": { "type": "on", "event": "invoked", "next": "go" },
                    "go": { "type": "warp", "to": { "var": "to" } },
                },
            }),
        )
        .row(
            "command",
            "warp",
            json!({ "script": path("script", "warp"), "inputs": ["to"] }),
        )
        .script(
            "commands",
            "command",
            json!({
                "sent": { "type": "on", "event": "invoked", "next": "list" },
                "list": { "type": "list_commands" },
            }),
        )
        .row(
            "command",
            "commands",
            json!({ "script": path("script", "commands") }),
        )
}

/// The server of the camp, running what takes a terminal's lines and runs its commands.
fn camp(content: Content) -> Sim {
    let mut sim = Sim::new(content);
    sim.serve("camp")
        .accept::<TerminalInput>()
        .accept::<TerminalCompletionRequest>()
        .message::<bevy_terminal::TerminalSaid>()
        .message::<bevy_terminal::TerminalCommanded>()
        .message::<ToClient<TerminalLine>>()
        .message::<ToClient<TerminalCompletions>>()
        .message::<ToClient<engine::systems::item::ItemShown>>()
        .add((
            chat::hear,
            command::run,
            command::retire,
            command::complete::answer,
        ))
        .plan(script::RunFlows::default());
    chat::route_lines(&mut sim.world);
    script::spawn_stateless(&mut sim.world);
    sim
}

/// The client sends `line` through its terminal, and the server takes it.
fn send(sim: &mut Sim, client: u32, line: &str) {
    let conn = sim.conn(client);
    sim.send(
        conn,
        TerminalInput {
            text: line.to_owned(),
        },
    );
    sim.tick();
}

/// The lines the server put in the client's terminal of late, oldest first.
fn lines(sim: &mut Sim, client: u32) -> Vec<(String, LineTone)> {
    let conn = sim.conn(client);
    sim.read(&mut MessageCursor::<ToClient<TerminalLine>>::default())
        .into_iter()
        .filter(|sent| sent.client == conn)
        .map(|sent| (sent.message.text, sent.message.tone))
        .collect()
}

/// How the server says `draft`, as the client has it typed, may go on.
fn suggested(sim: &mut Sim, client: u32, draft: &str) -> TerminalCompletions {
    let conn = sim.conn(client);
    sim.send(
        conn,
        TerminalCompletionRequest {
            text: draft.to_owned(),
        },
    );
    sim.tick();
    sim.read(&mut MessageCursor::<ToClient<TerminalCompletions>>::default())
        .into_iter()
        .filter(|sent| sent.client == conn)
        .map(|sent| sent.message)
        .next_back()
        .expect("an answer")
}

fn offered(sim: &mut Sim, client: u32, draft: &str) -> Vec<String> {
    suggested(sim, client, draft)
        .options
        .into_iter()
        .map(|option| option.text)
        .collect()
}

fn account(content: &Content, claims: &Value) -> AccountIdentity {
    let mut claims = claims.clone();
    claims["sub"] = json!("someone");
    AccountIdentity::of(
        content,
        serde_json::from_value::<TokenClaims>(claims).expect("claims"),
    )
}
