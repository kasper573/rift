use bevy_ecs::message::MessageCursor;
use bevy_ecs::prelude::*;
use bevy_replication::ToClient;
use bevy_terminal::{
    EMBED, LineKind, LineSpeaker, TerminalCompletionRequest, TerminalCompletions, TerminalEmbed,
    TerminalInput, TerminalLine,
};
use engine::core::content::Content;
use engine::core::net::auth::TokenClaims;
use engine::core::units::MeterPos;
use engine::systems::account::{AccountIdentity, RoleDef};
use engine::systems::chat::{ChatChannel, ChatEmbed, ChatEmbedKind, ChatLocation, ChatSpeaker};
use engine::systems::input::map::InputMap;
use engine::systems::player::{self, AreaDrawn};
use engine::systems::terminal::draft::ChatDraft;
use engine::systems::terminal::history::{InField, SentLines};
use engine::systems::{chat, command, movement, region};
use serde_json::{Value, json};
use testkit::{Game, Map, Sim, path};

#[test]
fn a_command_runs_its_script_on_what_was_typed_and_says_why_it_cannot() {
    let content = game().load();
    let mut sim = serving(content.clone(), "camp");
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
            Some(&(refusal.to_owned(), LineKind::Failed)),
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

    send(&mut sim, 1, "/join player 2");
    sim.tick();
    let (at, theirs) = (
        movement::position(&sim.world, me).expect("somewhere"),
        movement::position(&sim.world, friend).expect("somewhere"),
    );
    assert!(
        at.distance(theirs).0 < 1.5,
        "an actor is warped to where another stands: {at:?} by {theirs:?}"
    );
    let conn = sim.conn(1);
    sim.send(conn, AreaDrawn);

    send(&mut sim, 2, "/commands");
    sim.tick();
    let told = lines(&mut sim, 2);
    assert_eq!(
        told[told.len() - 6..],
        [
            "/commands",
            "/give <item>, [amount], [player]",
            "/join <player>",
            "/take <item>, [amount], [player]",
            "/visit <player>",
            "/warp <to>, [x], [y]"
        ]
        .map(|usage| (usage.to_owned(), LineKind::Notice))
    );

    let mut wood = serving(content, "wood");
    let away = wood.join(3);
    testkit::confer(&mut [&mut sim, &mut wood]);
    send(&mut sim, 1, "/give potion, 2, player 3");
    testkit::confer(&mut [&mut sim, &mut wood]);
    assert_eq!(
        wood.count(away, "potion"),
        2,
        "a global player is reached in whichever area they play"
    );

    send(&mut wood, 3, "/take potion, 5");
    wood.tick();
    let refusal = wood.errors(3);
    assert_eq!(
        refusal.len(),
        1,
        "taking more than the bag holds is refused"
    );
    sim.forget_notified();
    send(&mut sim, 1, "/take potion, 5, player 3");
    testkit::confer(&mut [&mut sim, &mut wood]);
    sim.tick();
    assert_eq!(
        (sim.errors(1), wood.count(away, "potion")),
        (refusal, 2),
        "what refuses it there is told to whoever sent it"
    );

    send(&mut sim, 1, "/visit player 3");
    assert_eq!(
        lines(&mut sim, 1).last(),
        Some(&("player 3 is no player".to_owned(), LineKind::Failed)),
        "a local player is one of this area alone"
    );

    send(&mut sim, 1, "/join player 3");
    let arrived = sim.hand_over(&mut wood);
    let (at, theirs) = (
        movement::position(&wood.world, arrived).expect("somewhere"),
        movement::position(&wood.world, away).expect("somewhere"),
    );
    assert!(
        at.distance(theirs).0 < 1.5,
        "and joined across areas: {at:?} by {theirs:?}"
    );
}

#[test]
fn a_global_player_is_handed_only_to_what_reaches_a_player_anywhere() {
    let refused = game()
        .edit("script", "join", |script| {
            script["nodes"]["go"] = json!({
                "type": "move_items",
                "from": { "var": "player" },
                "to": { "type": "subject" },
            });
        })
        .try_load()
        .err()
        .expect("refused");
    let rules: Vec<(&str, &str)> = refused
        .errors
        .iter()
        .map(|error| (error.file.as_str(), error.problem.rule))
        .collect();
    assert_eq!(
        rules,
        [(path("command", "join").as_str(), "wrong type")],
        "{refused}"
    );
}

#[test]
fn what_is_said_aloud_reaches_every_terminal_with_who_said_it_and_whether_they_typed_it() {
    let crier =
        json!([{ "type": "resident", "npc": "crier.npc.json", "at": { "x": 3.0, "y": 2.0 } }]);
    let content = Game::new()
        .area("camp", &Map::open(6, 6), crier)
        .hero("camp", 2.0, 2.0)
        .inputs()
        .npc(
            "crier",
            40.0,
            json!([{ "type": "script", "script": "crier_answers.script.json" }]),
        )
        .script(
            "crier_answers",
            "npc",
            json!({
                "heard": { "type": "on", "event": "heard", "words": ["ahoy"], "within": 4.0, "next": "answer" },
                "answer": { "type": "say", "text": ["Ahoy yourself."] },
            }),
        )
        .load();
    let inputs = InputMap::new(&content);
    let mut sim = serving(content, "camp");
    sim.world.insert_resource(inputs);
    region::hold_whole(&mut sim.world);
    sim.join(1);
    sim.join(2);
    sim.run(0.5);
    let listener = sim.conn(2);
    let mut heard = Vec::new();
    let mut cursor = MessageCursor::<ToClient<TerminalLine>>::default();

    send(&mut sim, 1, "Ahoy there!");
    for _ in 0..10 {
        sim.tick();
        heard.extend(
            sim.read(&mut cursor)
                .into_iter()
                .filter(|sent| sent.client == listener)
                .map(|sent| (sent.message.text, sent.message.kind)),
        );
    }

    let said = |text: &str, name: &str, typist| {
        let speaker = LineSpeaker {
            name: name.to_owned(),
            typist,
        };
        (text.to_owned(), LineKind::Said(speaker))
    };
    assert_eq!(
        heard,
        vec![
            said("Ahoy there!", "player 1", Some(1)),
            said("Ahoy yourself.", "crier", None),
        ],
        "a line names its speaker apart from its words, and only a player's names who typed it"
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

    let mut sim = serving(content.clone(), "camp");
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
            ["commands", "give ", "join ", "take ", "visit ", "warp "]
                .map(str::to_owned)
                .to_vec(),
            ["commands", "give ", "join ", "take ", "visit "]
                .map(str::to_owned)
                .to_vec()
        ),
        "a command is only offered to those who may send it"
    );
    send(&mut sim, 2, "/warp camp");
    assert_eq!(
        lines(&mut sim, 2).last(),
        Some(&("There is no command warp".to_owned(), LineKind::Failed)),
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
    let mut sim = serving(content.clone(), "camp");
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
    let mut wood = serving(content, "wood");
    wood.join(4);
    testkit::confer(&mut [&mut sim, &mut wood]);
    assert_eq!(
        suggested(&mut sim, 1, "/give plank, 2, 4").options,
        [option("player 4", "wood")],
        "a global player goes on to those of other areas too, each with the area they are in"
    );
    assert_eq!(
        (
            offered(&mut sim, 1, "/visit 4"),
            offered(&mut sim, 1, "/visit 3")
        ),
        (Vec::new(), vec!["player 3".to_owned()]),
        "a local player only to those of this area"
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
        muted: false,
    }
}

/// A camp and a wood, a potion, and five commands: one giving an item to a player anywhere, one
/// warping to a place, one warping to a player anywhere, one warping to a player of the same
/// area, one listing the commands.
fn game() -> Game {
    Game::new()
        .area("camp", &Map::open(6, 6), json!([]))
        .area("wood", &Map::open(6, 6), json!([]))
        .hero("camp", 2.0, 2.0)
        .sfx("ui_refuse")
        .item("potion", json!({ "kind": { "type": "resource" } }))
        .row(
            "script",
            "give",
            json!({
                "runs_on": "command",
                "variables": {
                    "item": { "type": "row", "table": "item" },
                    "amount": { "type": "integer", "default": 1 },
                    "player": { "type": "global_player", "default": { "type": "subject" } },
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
            "take",
            json!({
                "runs_on": "command",
                "variables": {
                    "item": { "type": "row", "table": "item" },
                    "amount": { "type": "integer", "default": 1 },
                    "player": { "type": "global_player", "default": { "type": "subject" } },
                },
                "nodes": {
                    "sent": { "type": "on", "event": "invoked", "next": "hand" },
                    "hand": {
                        "type": "items",
                        "who": { "var": "player" },
                        "take": [{ "item": { "var": "item" }, "count": { "var": "amount" } }],
                    },
                },
            }),
        )
        .row(
            "command",
            "take",
            json!({ "script": path("script", "take"), "inputs": ["item", "amount", "player"] }),
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
        .row(
            "script",
            "join",
            json!({
                "runs_on": "command",
                "variables": { "player": { "type": "global_player" } },
                "nodes": {
                    "sent": { "type": "on", "event": "invoked", "next": "go" },
                    "go": { "type": "warp_to_actor", "to": { "var": "player" } },
                },
            }),
        )
        .row(
            "command",
            "join",
            json!({ "script": path("script", "join"), "inputs": ["player"] }),
        )
        .row(
            "script",
            "visit",
            json!({
                "runs_on": "command",
                "variables": { "player": { "type": "local_player" } },
                "nodes": {
                    "sent": { "type": "on", "event": "invoked", "next": "go" },
                    "go": { "type": "warp_to_actor", "to": { "var": "player" } },
                },
            }),
        )
        .row(
            "command",
            "visit",
            json!({ "script": path("script", "visit"), "inputs": ["player"] }),
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

#[test]
fn a_player_says_only_so_many_lines_in_a_while_and_is_told_when_one_is_too_many() {
    let mut sim = serving(game().load(), "camp");
    sim.join(1);
    sim.join(2);
    let (typist, listener) = (sim.conn(1), sim.conn(2));
    let mut cursor = MessageCursor::<ToClient<TerminalLine>>::default();
    let mut shown = |sim: &mut Sim, text: &str| -> Vec<(Entity, LineKind)> {
        send(sim, 1, text);
        sim.read(&mut cursor)
            .into_iter()
            .map(|sent| (sent.client, sent.message.kind))
            .collect()
    };

    for line in 1..=5 {
        let heard = shown(&mut sim, &format!("line {line}"));
        assert!(
            heard.iter().any(|(conn, _)| *conn == listener),
            "line {line}"
        );
    }
    assert_eq!(
        shown(&mut sim, "line 6"),
        [(typist, LineKind::Failed)],
        "a line too many is said to nobody, and whoever typed it is told so alone"
    );
    assert_eq!(
        lines(&mut sim, 1).last(),
        Some(&("You are writing too fast".to_owned(), LineKind::Failed))
    );

    send(&mut sim, 1, "/give potion");
    sim.tick();
    let me = sim.player(1);
    assert_eq!(sim.count(me, "potion"), 1, "a command is no line");

    sim.run(10.0);
    let heard = shown(&mut sim, "line 7");
    assert!(
        heard.iter().any(|(conn, _)| *conn == listener),
        "once the while is over they speak again"
    );
}

#[test]
fn a_line_reaches_its_hearers_with_the_things_set_in_it_unless_one_is_no_thing_of_the_game() {
    let mut sim = serving(game().load(), "camp");
    sim.join(1);
    sim.join(2);
    let listener = sim.conn(2);
    let mut cursor = MessageCursor::<ToClient<TerminalLine>>::default();
    let mut heard =
        |sim: &mut Sim, embeds: Vec<TerminalEmbed>| -> Vec<(String, Vec<TerminalEmbed>)> {
            send_holding(sim, 1, &format!("who wants a {EMBED}?"), embeds);
            sim.read(&mut cursor)
                .into_iter()
                .filter(|sent| sent.client == listener)
                .map(|sent| (sent.message.text, sent.message.embeds))
                .collect()
        };
    let thing = |kind: &str, key: &str| TerminalEmbed {
        kind: kind.to_owned(),
        key: key.to_owned(),
    };
    let potion = thing("item", "potion.item.json");

    assert_eq!(
        heard(&mut sim, vec![potion.clone()]),
        vec![(format!("who wants a {EMBED}?"), vec![potion])],
        "a hearer is told what the line holds where the line marks it"
    );
    for unknown in [
        thing("item", "elixir.item.json"),
        thing("quest", "potion.item.json"),
        thing("spell", "potion.item.json"),
    ] {
        assert_eq!(
            heard(&mut sim, vec![unknown.clone()]),
            Vec::new(),
            "a line holding {unknown:?}, which the game does not have, reaches nobody"
        );
    }

    let content = sim.world.resource::<Content>().clone();
    let speaker = sim.player(1);
    let stands = movement::position(&sim.world, speaker).expect("somewhere");
    let elsewhere = ChatEmbed::Location(ChatLocation {
        area: content.by_path(&path("area", "wood")).expect("the wood"),
        x: 4,
        y: 4,
    });
    let told: Vec<Option<ChatEmbed>> = heard(&mut sim, vec![elsewhere.sent(&content)])
        .iter()
        .flat_map(|(_, held)| held)
        .map(|sent| ChatEmbed::read(&content, sent))
        .collect();
    assert_eq!(
        told,
        [Some(ChatEmbed::Location(ChatLocation {
            area: content.by_path(&path("area", "camp")).expect("the camp"),
            x: stands.x.round() as i32,
            y: stands.y.round() as i32,
        }))],
        "a location a line holds is where its speaker stands, whatever their client sent"
    );
}

#[test]
fn a_thing_set_in_a_line_being_typed_is_held_whole_and_sent_as_a_mark_in_its_place() {
    let content = game().load();
    let potion =
        ChatEmbed::of(&content, ChatEmbedKind::Item, "potion.item.json").expect("the potion");
    let held = |draft: &ChatDraft| draft.held().collect::<Vec<_>>();
    let mut draft = ChatDraft::default();
    draft.write("selling ".to_owned());

    assert_eq!(draft.add(potion, "Potion"), 14, "the caret goes after it");
    assert_eq!(draft.text(), "selling Potion");

    assert_eq!(draft.retyped("selling Potion cheap", 20), None);
    assert_eq!(draft.retyped("now selling Potion cheap", 4), None);
    assert_eq!(
        held(&draft),
        vec![(12..18, potion)],
        "typing round a thing leaves it be, wherever that moves it"
    );
    assert_eq!(
        [12, 14, 18].map(|caret| draft.kept_out(caret)),
        [12, 18, 18],
        "a caret rests before a thing or after it, never inside its name"
    );
    let sent = TerminalEmbed {
        kind: "item".to_owned(),
        key: "potion.item.json".to_owned(),
    };
    assert_eq!(
        draft.sent(&content),
        (format!("now selling {EMBED} cheap"), vec![sent]),
        "a line is sent with a mark where each thing stands, and the things in their order"
    );

    assert_eq!(
        draft.retyped("now selling Potio cheap", 17),
        Some(12),
        "erasing right after a thing takes the whole of it out, and the caret to where it stood"
    );
    assert_eq!(draft.text(), "now selling  cheap");
    assert_eq!(held(&draft), Vec::new());
    assert_eq!(
        draft.sent(&content),
        ("now selling  cheap".to_owned(), Vec::new())
    );
}

#[test]
fn the_recall_keys_step_through_the_lines_sent_and_give_back_the_one_being_typed() {
    let content = game().load();
    let potion =
        ChatEmbed::of(&content, ChatEmbedKind::Item, "potion.item.json").expect("the potion");
    let line = |text: &str| {
        let mut draft = ChatDraft::default();
        draft.write(text.to_owned());
        draft
    };
    let mut selling = line("selling ");
    selling.add(potion, "Potion");
    let mut sent = SentLines::default();
    let typing = InField {
        draft: line("meet at the do"),
        caret: 6,
    };
    assert_eq!(
        sent.earlier(&typing),
        None,
        "nothing to bring back before a line is sent"
    );

    for draft in [
        line("hello"),
        selling.clone(),
        line(" "),
        selling.clone(),
        line("/who"),
    ] {
        sent.keep(draft, 3);
    }
    let shown = sent.earlier(&typing).expect("the newest line");
    assert_eq!(
        (shown.draft.text(), shown.caret),
        ("/who", 4),
        "the caret at its end"
    );
    assert_eq!(
        sent.place(),
        Some((1, 3)),
        "a blank line and a line sent twice are kept once"
    );
    let shown = sent.earlier(&shown).expect("an earlier line");
    assert_eq!(
        shown.draft, selling,
        "a line comes back holding what it held"
    );
    assert_eq!(
        sent.earlier(&shown).map(|shown| shown.draft),
        Some(line("hello"))
    );
    assert!(!sent.has_earlier());
    assert_eq!(
        sent.earlier(&shown),
        None,
        "the oldest line is as far back as it goes"
    );

    assert_eq!(sent.later().map(|shown| shown.draft), Some(selling.clone()));
    assert_eq!(sent.later().map(|shown| shown.draft), Some(line("/who")));
    assert_eq!(
        sent.later(),
        Some(typing.clone()),
        "past the newest line, the line being typed comes back as it was, caret and all"
    );
    assert_eq!(sent.later(), None);

    sent.earlier(&typing);
    assert_eq!(
        sent.give_back(),
        Some(typing.clone()),
        "a dismiss gives it back at once"
    );
    assert!(!sent.browsing());

    sent.earlier(&typing);
    sent.keep(line("/who is near"), 3);
    assert_eq!(
        sent.place(),
        None,
        "sending a line brought back and edited ends the browsing"
    );
    let back: Vec<String> =
        std::iter::successors(sent.earlier(&typing), |shown| sent.earlier(shown))
            .map(|shown| shown.draft.text().to_owned())
            .collect();
    assert_eq!(
        back,
        ["/who is near", "/who", "selling Potion"],
        "an edit is kept as a line of its own, the line it came from as it was, and the oldest go"
    );
}

/// The server of one of the game's areas, running what takes a terminal's lines and runs its
/// commands.
fn serving(content: Content, area: &str) -> Sim {
    let mut sim = Sim::new(content);
    sim.serve(area)
        .accept::<TerminalInput>()
        .accept::<TerminalCompletionRequest>()
        .accept::<AreaDrawn>()
        .message::<bevy_terminal::TerminalSaid>()
        .message::<bevy_terminal::TerminalCommanded>()
        .message::<ToClient<TerminalLine>>()
        .message::<ToClient<ChatSpeaker>>()
        .message::<ToClient<TerminalCompletions>>()
        .message::<ToClient<engine::systems::item::ItemShown>>()
        .add((
            chat::hear,
            chat::speak,
            command::run,
            command::retire,
            command::complete::answer,
            player::arrival_holds,
        ))
        .plan(engine::core::script::run::FlowRunner::default());
    chat::route_lines(&mut sim.world);
    engine::systems::spawn_scripts(&mut sim.world);
    sim
}

/// The client sends `line` through its terminal, and the server takes it.
fn send(sim: &mut Sim, client: u32, line: &str) {
    send_holding(sim, client, line, Vec::new());
}

/// The client sends `line` through its terminal with `embeds` set at its marks, and the server
/// takes it.
fn send_holding(sim: &mut Sim, client: u32, line: &str, embeds: Vec<TerminalEmbed>) {
    let conn = sim.conn(client);
    sim.send(
        conn,
        TerminalInput {
            text: line.to_owned(),
            channel: ChatChannel::Area.terminal(),
            embeds,
        },
    );
    sim.tick();
}

/// The lines the server put in the client's terminal of late, oldest first.
fn lines(sim: &mut Sim, client: u32) -> Vec<(String, LineKind)> {
    let conn = sim.conn(client);
    sim.read(&mut MessageCursor::<ToClient<TerminalLine>>::default())
        .into_iter()
        .filter(|sent| sent.client == conn)
        .map(|sent| (sent.message.text, sent.message.kind))
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
