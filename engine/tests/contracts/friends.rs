use bevy_ecs::message::MessageCursor;
use bevy_replication::ToClient;
use engine::core::content::Content;
use engine::systems::GameSettings;
use engine::systems::area::AreaDef;
use engine::systems::friend::{self, AskFriendship, FindPlayers, PlayersFound, RemoveFriend};
use engine::systems::player;
use engine::systems::player::snapshot::{CharacterSnapshot, JoinAs, JoinOrigin};
use engine::systems::social_request::RequestAnswer;
use serde_json::json;
use testkit::{Game, Map};

use crate::server::Server;

#[test]
fn friends_are_made_by_a_request_and_parted_by_either() {
    let mut server = Server::of(game(100, 20), vec![friend::request_source()]);
    server.sign_in(1, "Wren", "village");
    server.sign_in(2, "Hale", "forest");

    befriend(&mut server, 1, "Hale");
    assert_eq!(server.told(1), ["Friend request sent to Hale"]);
    assert_eq!(
        server.cards(2),
        [("Wren wants to be your friend".to_owned(), false)],
        "it reaches the player asked in whatever world they are"
    );
    server.answer(2, RequestAnswer::Deny);
    server.step();
    befriend(&mut server, 1, "Hale");
    assert_eq!(
        (server.cards(1), server.cards(2)),
        (
            vec![("You asked Hale to be your friend".to_owned(), true)],
            vec![]
        ),
        "a denial is quiet: the asker sees their request wait on, and cannot ask again while it does"
    );
    assert_eq!(server.told(1), ["Friend request sent to Hale"]);
    assert_eq!(
        (friends(&mut server, 1), friends(&mut server, 2)),
        (vec![], vec![])
    );

    befriend(&mut server, 2, "Wren");
    assert_eq!(
        (friends(&mut server, 1), friends(&mut server, 2)),
        (vec!["Hale".to_owned()], vec!["Wren".to_owned()]),
        "the one who denied asks in turn, and the two are friends at once, on both lists"
    );
    assert_eq!((server.cards(1), server.cards(2)), (vec![], vec![]));
    assert_eq!(
        server.told(1),
        ["Friend request sent to Hale", "Hale is now your friend"]
    );
    befriend(&mut server, 1, "Hale");
    assert_eq!(server.village.errors(1), ["Hale is already your friend"]);

    let forest = server.forest.id::<AreaDef>("forest");
    assert_eq!(
        server.view(1).friends[0].online_in,
        Some(forest),
        "a friend sees where their friend plays"
    );
    server.forest.disconnect(2);
    let left_at = server.now();
    server.step();
    let hale = server.view(1).friends[0].clone();
    assert_eq!(
        (hale.online_in, hale.last_seen),
        (None, left_at),
        "and when they were last seen, once their player is gone"
    );
    server.sign_in(2, "Hale", "forest");
    server.step();
    assert_eq!(server.view(1).friends[0].online_in, Some(forest));

    let told = server.told(2);
    server.send(1, RemoveFriend(server.number("Hale")));
    server.step();
    assert_eq!(
        (friends(&mut server, 1), friends(&mut server, 2)),
        (vec![], vec![]),
        "either ends it for both"
    );
    assert_eq!(server.told(2), told, "and the other is not told");

    befriend(&mut server, 1, "Hale");
    server.answer(2, RequestAnswer::Accept);
    server.step();
    assert_eq!(friends(&mut server, 2), ["Wren"]);
    server.block(2, "Wren", true);
    server.step();
    assert_eq!(
        (friends(&mut server, 1), friends(&mut server, 2)),
        (vec![], vec![]),
        "blocking a friend ends the friendship"
    );
}

#[test]
fn a_friends_list_and_the_requests_an_account_has_waiting_have_their_limits() {
    let mut server = Server::of(game(3, 2), vec![friend::request_source()]);
    for (client, name) in [
        (1, "Wren"),
        (2, "Hale"),
        (3, "Marrow"),
        (4, "Osric"),
        (5, "Isolde"),
    ] {
        server.sign_in(client, name, "village");
    }
    befriend(&mut server, 1, "Hale");
    befriend(&mut server, 1, "Marrow");
    befriend(&mut server, 1, "Osric");
    assert_eq!(
        (server.village.errors(1), server.cards(4)),
        (
            vec!["You have too many friend requests waiting".to_owned()],
            vec![]
        ),
        "an account has only so many requests waiting"
    );

    server.answer(2, RequestAnswer::Accept);
    server.step();
    befriend(&mut server, 1, "Osric");
    assert_eq!(
        server.cards(4).len(),
        1,
        "an answer makes room for the next"
    );
    server.village.forget_notified();
    befriend(&mut server, 1, "Isolde");
    assert_eq!(
        (server.village.errors(1), server.cards(5)),
        (vec!["Your friends list is full".to_owned()], vec![]),
        "a list holds only so many, counting those asked: accepting always finds the room it was \
         asked with"
    );

    befriend(&mut server, 5, "Wren");
    assert_eq!(
        (server.told(5), server.village.errors(5)),
        (vec!["Friend request sent to Wren".to_owned()], vec![]),
        "asking someone whose list is full looks like any other request"
    );
    server.village.forget_notified();
    let asks = server.view(1).requests.iter().position(|card| !card.mine);
    let asks = server.view(1).requests[asks.expect("Isolde's request")].id;
    let accept = engine::systems::social_request::AnswerSocialRequest {
        request: asks,
        answer: RequestAnswer::Accept,
    };
    server.send(1, accept);
    server.step();
    assert_eq!(
        (
            server.village.errors(1),
            friends(&mut server, 5),
            server.cards(5).len()
        ),
        (vec!["Your friends list is full".to_owned()], vec![], 1),
        "it cannot be accepted until there is room, and waits on"
    );
    server.send(1, RemoveFriend(server.number("Hale")));
    server.step();
    server.send(1, accept);
    server.step();
    assert_eq!(friends(&mut server, 5), ["Wren"]);
}

#[test]
fn a_player_is_found_by_the_start_of_their_name() {
    let mut server = Server::of(game(100, 20), vec![friend::request_source()]);
    for (client, name) in [(1, "Wren"), (2, "Hanne"), (3, "halvard"), (4, "Hale")] {
        server.sign_in(client, name, "village");
    }
    let content = server.village.world.resource::<Content>().clone();
    let staged = JoinAs {
        snapshot: CharacterSnapshot::of_player(&content, "Zofia", player::def(&content)),
        start: content.built::<GameSettings>().initial_spawn,
        origin: JoinOrigin::Staged,
    };
    server.sign_in_as(5, "Osric", "village", Some(staged));
    server.village.disconnect(4);
    server.later(31.0);
    server.block(2, "Wren", true);
    server.step();
    let mut heard = MessageCursor::default();
    let mut find = |server: &mut Server, typed: &str| -> (Vec<String>, u32) {
        server.send(1, FindPlayers(typed.to_owned()));
        server.step();
        let conn = server.village.conn(1);
        let found: Vec<ToClient<PlayersFound>> = server.village.read(&mut heard);
        let [found] = found.as_slice() else {
            panic!("one answer to {typed:?}, not {}", found.len());
        };
        assert_eq!((found.client, found.message.starts.as_str()), (conn, typed));
        let names = found
            .message
            .players
            .iter()
            .map(|player| player.name.clone());
        (names.collect(), found.message.in_all)
    };
    assert_eq!(
        find(&mut server, "ha"),
        (vec!["Hale".to_owned(), "halvard".to_owned()], 3),
        "whatever its case, whoever they are, playing or not: the first few by name, of all \
         there are"
    );
    assert_eq!(find(&mut server, "HAN"), (vec!["Hanne".to_owned()], 1));
    assert_eq!(find(&mut server, "wr"), (vec!["Wren".to_owned()], 1));
    assert_eq!(
        find(&mut server, "h"),
        (vec![], 0),
        "one letter is no search"
    );
    assert_eq!(find(&mut server, "zz"), (vec![], 0));
    assert_eq!(
        (find(&mut server, "zo"), find(&mut server, "os")),
        ((vec!["Zofia".to_owned()], 1), (vec![], 0)),
        "a player goes by the name their character plays under, whatever they signed in as"
    );
}

/// The player of `client` asks the player called `name` to be their friend.
fn befriend(server: &mut Server, client: u32, name: &str) {
    server.send(client, AskFriendship(server.number(name)));
    server.step();
}

/// The names on the friends list of the player of `client`.
fn friends(server: &mut Server, client: u32) -> Vec<String> {
    let view = server.view(client);
    view.friends
        .into_iter()
        .map(|friend| friend.player.name)
        .collect()
}

/// A game of two areas whose friends lists hold `most`, with `requests` waiting at the most,
/// and whose searches list two players at once.
fn game(most: u32, requests: u32) -> Game {
    Game::new()
        .area("village", &Map::open(9, 9), json!([]))
        .area("forest", &Map::open(9, 9), json!([]))
        .hero("village", 4.0, 4.0)
        .sfx("ui_refuse")
        .edit_settings(|settings| {
            settings["lingers_for"] = json!(30.0);
            settings["suggestions"] = json!(2);
            settings["friends"] =
                json!({ "most": most, "requests": requests, "request_lasts": 604_800.0 });
        })
}
