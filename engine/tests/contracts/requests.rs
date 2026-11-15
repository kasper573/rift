use bevy_ecs::prelude::*;
use engine::core::units::Seconds;
use engine::systems::GameSettings;
use engine::systems::friend;
use engine::systems::player::Xp;
use engine::systems::social_request::{
    AcceptsRequestsFrom, ChooseAcceptsRequests, RequestAnswer, RequestUnaccepted, SocialRequest,
    SocialRequestKind, SocialRequestSource, SocialRequestWords,
};
use engine::systems::society::Settling;
use serde_json::json;
use testkit::{Game, Map};

use crate::saves::{Saves, checkpoint};
use crate::server::Server;

#[test]
fn a_request_waits_on_both_players_cards_until_it_is_answered_taken_back_or_lapses() {
    let mut server = Server::of(game(), sources());
    let wren = server.sign_in(1, "Wren", "village");
    server.sign_in(2, "Hale", "forest");

    server.ask(1, SocialRequestKind::Seat, "Hale");
    let asked_at = server.now();
    server.step();
    assert_eq!(
        server.cards(1),
        [("You asked Hale for a seat".to_owned(), true)],
        "the asker sees what they asked"
    );
    assert_eq!(
        server.cards(2),
        [("Wren asks for a seat on your Rowboat".to_owned(), false)],
        "and the player asked sees who asks, wherever they are"
    );
    let waits = server.view(2).requests[0].clone();
    assert_eq!(
        (waits.other.name.as_str(), waits.until, waits.shown_until),
        ("Wren", asked_at.after(Seconds(20.0)), waits.until),
        "a seat waits as long as its card shows"
    );
    server.ask(1, SocialRequestKind::Seat, "Hale");
    server.step();
    assert_eq!(
        server.view(2).requests,
        std::slice::from_ref(&waits),
        "asking again while it waits changes nothing"
    );

    server.answer(1, RequestAnswer::Accept);
    server.step();
    assert_eq!(server.cards(2).len(), 1, "only the player asked says yes");
    server.answer(2, RequestAnswer::Accept);
    server.step();
    assert_eq!((server.cards(1), server.cards(2)), (vec![], vec![]));
    assert_eq!(rides(&server, wren), 1, "a yes does what the kind does");
    assert_eq!(server.told(1), ["Hale let you aboard"]);

    server.ask(1, SocialRequestKind::Seat, "Hale");
    server.step();
    server.answer(2, RequestAnswer::Accept);
    server.step();
    assert_eq!(
        rides(&server, wren),
        1,
        "a yes the kind refuses does nothing"
    );
    for client in [1, 2] {
        assert_eq!(
            server.village_or_forest(client).errors(client),
            ["Every seat is taken"],
            "and both are told why"
        );
    }

    server.ask(2, SocialRequestKind::Seat, "Wren");
    server.step();
    server.answer(1, RequestAnswer::Decline);
    server.step();
    assert_eq!((server.cards(1), server.cards(2)), (vec![], vec![]));
    assert_eq!(
        server.told(2),
        ["Wren said no"],
        "a no is told to the asker"
    );

    server.ask(2, SocialRequestKind::Seat, "Wren");
    server.step();
    server.answer(2, RequestAnswer::TakeBack);
    server.step();
    assert_eq!(
        (server.cards(1), server.cards(2)),
        (vec![], vec![]),
        "the asker takes it back"
    );

    server.ask(2, SocialRequestKind::Seat, "Wren");
    server.step();
    server.later(21.0);
    assert_eq!(
        (server.cards(1), server.cards(2)),
        (vec![], vec![]),
        "it is gone when its wait is over"
    );
    assert_eq!(server.told(2), ["Wren said no"], "and nobody is told");

    server.ask(1, SocialRequestKind::Friendship, "Hale");
    server.ask(2, SocialRequestKind::Friendship, "Wren");
    server.step();
    assert_eq!(
        (server.cards(1), server.cards(2)),
        (vec![], vec![]),
        "two who ask each other for the same thing have both said yes"
    );
    assert_eq!(
        server.told(1),
        [
            "Hale let you aboard",
            "Friend request sent to Hale",
            "Hale is now your friend"
        ],
        "the one whose request was answered is told"
    );
}

#[test]
fn every_request_passes_one_gate() {
    let mut server = Server::of(game(), sources());
    server.sign_in(1, "Wren", "village");
    server.sign_in(2, "Hale", "village");
    server.sign_in(3, "Marrow", "village");
    let refused = |server: &mut Server, why: &str, case: &str| {
        server.village.forget_notified();
        server.ask(1, SocialRequestKind::Seat, "Hale");
        server.step();
        assert_eq!(server.village.errors(1), [why], "{case}");
        assert_eq!(server.cards(1), [], "{case}");
    };

    server.block(2, "Wren", true);
    server.step();
    refused(
        &mut server,
        "Hale is not taking requests",
        "the player asked keeps the asker away",
    );
    server.block(2, "Wren", false);
    server.block(1, "Hale", true);
    server.step();
    refused(
        &mut server,
        "You have blocked Hale",
        "the asker keeps the player asked away",
    );
    server.block(1, "Hale", false);
    for from in [
        AcceptsRequestsFrom::Nobody,
        AcceptsRequestsFrom::Friends,
        AcceptsRequestsFrom::Party,
    ] {
        server.send(2, ChooseAcceptsRequests(from));
        server.step();
        assert_eq!(server.view(2).accepts, from);
        let case = format!("the player asked takes requests from {from:?}, and the asker is none");
        refused(&mut server, "Hale is not taking requests", &case);
    }
    server.send(2, ChooseAcceptsRequests(AcceptsRequestsFrom::Friends));
    server.ask(2, SocialRequestKind::Friendship, "Wren");
    server.step();
    server.answer(1, RequestAnswer::Accept);
    server.step();
    server.ask(1, SocialRequestKind::Seat, "Hale");
    server.step();
    assert_eq!(
        server.cards(2).len(),
        1,
        "the player asked takes requests from their friends, and the asker is one"
    );
    server.answer(2, RequestAnswer::Decline);
    server.send(2, ChooseAcceptsRequests(AcceptsRequestsFrom::Everyone));
    server.step();

    server.village.disconnect(2);
    server.step();
    refused(
        &mut server,
        "Hale is not here",
        "the player asked is away from their character, and a seat needs them at it",
    );
    server.later(31.0);
    refused(&mut server, "Hale is not here", "the player asked is gone");
    server.village.forget_notified();
    server.ask(3, SocialRequestKind::Friendship, "Hale");
    server.step();
    assert_eq!(
        (server.village.errors(3), server.cards(3).len()),
        (vec![], 1),
        "friendship reaches a player who is gone"
    );
    server.sign_in(2, "Hale", "village");
    server.step();
    assert_eq!(
        server.cards(2),
        [("Marrow wants to be your friend".to_owned(), false)],
        "and waits for them"
    );

    server.ask(2, SocialRequestKind::Seat, "Wren");
    server.step();
    assert_eq!(server.cards(1).len(), 1, "otherwise it is sent");
}

#[test]
fn a_blocked_player_is_kept_away_until_it_is_undone() {
    let mut server = Server::of(game(), sources());
    server.sign_in(1, "Wren", "village");
    server.sign_in(2, "Hale", "village");
    server.sign_in(3, "Marrow", "village");
    server.ask(2, SocialRequestKind::Seat, "Wren");
    server.ask(1, SocialRequestKind::Friendship, "Hale");
    server.ask(3, SocialRequestKind::Seat, "Wren");
    server.step();
    assert_eq!((server.cards(1).len(), server.cards(2).len()), (3, 2));

    server.block(1, "Hale", true);
    server.step();
    assert_eq!(
        (server.cards(1), server.cards(2)),
        (
            vec![("Marrow asks for a seat on your Rowboat".to_owned(), false)],
            vec![]
        ),
        "every request between the two is gone, whoever asked, and no other"
    );
    let blocked = |server: &mut Server, client| -> Vec<String> {
        let view = server.view(client);
        view.blocked.into_iter().map(|player| player.name).collect()
    };
    assert_eq!(blocked(&mut server, 1), ["Hale"]);
    assert_eq!(blocked(&mut server, 2), Vec::<String>::new());
    assert_eq!(
        (server.told(2), server.village.errors(2)),
        (vec![], vec![]),
        "the blocked player is not told"
    );
    server.ask(2, SocialRequestKind::Friendship, "Wren");
    server.step();
    assert_eq!(
        server.village.errors(2),
        ["Wren is not taking requests"],
        "they are refused as anyone a setting keeps out is"
    );

    server.block(1, "Hale", false);
    server.step();
    assert_eq!(blocked(&mut server, 1), Vec::<String>::new());
    server.ask(2, SocialRequestKind::Friendship, "Wren");
    server.step();
    assert_eq!(server.cards(1).len(), 2, "undone, they may ask again");
}

#[test]
fn what_an_account_set_blocked_befriended_and_was_asked_is_kept_from_one_server_to_the_next() {
    let saves = Saves::in_a_file();
    let content = game().load();
    let mut server = Server::saving(&content, &saves, sources());
    server.sign_in(1, "Wren", "village");
    server.sign_in(2, "Hale", "village");
    server.sign_in(3, "Marrow", "village");
    server.sign_in(4, "Osric", "forest");
    server.step();
    server.ask(4, SocialRequestKind::Friendship, "Wren");
    server.step();
    server.answer(1, RequestAnswer::Accept);
    server.step();
    server.send(1, ChooseAcceptsRequests(AcceptsRequestsFrom::Nobody));
    server.block(1, "Marrow", true);
    server.ask(1, SocialRequestKind::Friendship, "Hale");
    server.ask(1, SocialRequestKind::Seat, "Hale");
    server.step();
    let friendship = server.cards(2)[..1].to_vec();
    let report = checkpoint(
        &mut [&mut server.village, &mut server.forest],
        &mut server.above,
    );
    assert!(report.outcome.is_ok());

    let mut next = Server::saving(&content, &saves, sources());
    next.village.later(60.0);
    next.sign_in(1, "Wren", "village");
    next.sign_in(2, "Hale", "village");
    next.step();
    let wren = next.view(1);
    assert_eq!(wren.accepts, AcceptsRequestsFrom::Nobody);
    let blocked: Vec<&str> = wren
        .blocked
        .iter()
        .map(|player| player.name.as_str())
        .collect();
    assert_eq!(blocked, ["Marrow"]);
    let friends: Vec<&str> = wren
        .friends
        .iter()
        .map(|friend| friend.player.name.as_str())
        .collect();
    assert_eq!(friends, ["Osric"]);
    assert_eq!(
        next.cards(2),
        friendship,
        "a request that reaches a player who is gone waits over a restart, and no other"
    );
}

/// How many rides `character` was given.
fn rides(server: &Server, character: Entity) -> u32 {
    server
        .village
        .world
        .get::<Xp>(character)
        .map_or(0, |xp| xp.amount)
}

/// The kinds of request the tests ask: a seat, which needs both players here and is given once,
/// and friendship, which reaches a player who is gone.
fn sources() -> Vec<SocialRequestSource> {
    fn seated(
        settling: &mut Settling,
        _: &GameSettings,
        request: &SocialRequest,
    ) -> Result<(), RequestUnaccepted> {
        let gone = || RequestUnaccepted::Over("The asker is gone".to_owned());
        let (world, asker) = settling.reach(&request.asker).ok_or_else(gone)?;
        let mut rides = world.get_mut::<Xp>(asker).ok_or_else(gone)?;
        if rides.amount > 0 {
            return Err(RequestUnaccepted::Over("Every seat is taken".to_owned()));
        }
        rides.gain(1);
        Ok(())
    }
    vec![
        SocialRequestSource {
            kind: SocialRequestKind::Seat,
            lasts: |settings| Some(settings.requests.card_shows),
            reaches_offline: false,
            words: |_| SocialRequestWords {
                asks: "{name} asks for a seat on your {about}",
                asked: "You asked {name} for a seat",
                sent: None,
                accepted: Some("{name} let you aboard"),
                declined: Some("{name} said no"),
            },
            may_send: |_, _, _, _| Ok(()),
            accepted: seated,
        },
        friend::request_source(),
    ]
}

fn game() -> Game {
    Game::new()
        .area("village", &Map::open(9, 9), json!([]))
        .area("forest", &Map::open(9, 9), json!([]))
        .hero("village", 4.0, 4.0)
        .sfx("ui_refuse")
        .edit_settings(|settings| {
            settings["lingers_for"] = json!(30.0);
            settings["friends"] =
                json!({ "most": 100, "requests": 20, "request_lasts": 604_800.0 });
        })
}
