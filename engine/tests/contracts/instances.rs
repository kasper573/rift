use bevy_ecs::prelude::*;
use engine::core::content::Content;
use engine::core::math::Pos;
use engine::core::time::WallClock;
use engine::core::units::{MeterPos, Meters, Seconds};
use engine::systems::area::Place;
use engine::systems::area::{self, AreaDef, AreaId, transition};
use engine::systems::area_instance::entrance::{
    EntranceAnswer, EntranceChoice, EntranceParty, EntranceView,
};
use engine::systems::area_instance::roster::{InstanceCallShown, RosterRowShown};
use engine::systems::area_instance::{
    self, AreaInstanceGiven, AreaInstanceShown, AreaInstanceWorld, CallReply, CallRequest,
    LeaveInstance, RosterPlace,
};
use engine::systems::dialogue::{self, ConversationRequest};
use engine::systems::interact::{self, InteractRequest};
use engine::systems::item::{self, Inventory, ItemDef, ItemStack, UseItemRequest};
use engine::systems::mail::{InboxSender, InboxView};
use engine::systems::movement::{self, position};
use engine::systems::npc::{self, NpcDef};
use engine::systems::party::PartyRequest;
use engine::systems::player::snapshot::{CharacterStart, JoinAs, JoinOrigin, SavedCharacter};
use engine::systems::player::{self, AreaDrawn, ClientId, Players, SpawnPoint, Xp};
use engine::systems::prompt;
use engine::systems::social_request::RequestAnswer;
use engine::systems::society::SocietyView;
use engine::systems::{combat, request_sources, stat, window};
use serde_json::{Value, json};
use testkit::{Game, Map, Sim, path};

use crate::saves::confer;
use crate::server::{Server, serving};

const ENTRANCE: Pos<Meters> = Pos::new(3.0, 3.0);
const BY_THE_DOOR: Pos<Meters> = Pos::new(9.0, 6.0);
const DOOR: Pos<Meters> = Pos::new(6.0, 9.0);

#[test]
fn a_player_starts_a_copy_at_an_entrance_panel_alone_or_with_a_party() {
    let mut host = Host::of(game(3), 2);
    for (client, name) in [
        (1, "Wren"),
        (2, "Hale"),
        (3, "Marrow"),
        (4, "Osric"),
        (5, "Tamsin"),
    ] {
        host.sign_in(client, name);
    }

    uses(&mut host, 4, "vault_key");
    assert_eq!(
        (
            host.entrance(4).map(|panel| panel.party),
            host.run_of(4).is_none()
        ),
        (Some(EntranceParty::Alone), true),
        "the panel opens for a player in no party too, and lets nobody in by itself"
    );
    host.carry(4, &[host.planks(2)]);
    host.choose_with(4, EntranceAnswer::GoAlone, vec![host.planks(1)]);
    assert_eq!(
        (
            host.run_of(4).is_none(),
            host.entrance(4).is_some(),
            host.count(4, "plank"),
            host.errors(4).len()
        ),
        (true, true, 2, 1),
        "the wrong items are refused with a reason, nothing is taken, and the panel stays open"
    );
    host.choose_with(4, EntranceAnswer::GoAlone, vec![host.planks(3)]);
    assert_eq!(
        (host.run_of(4).is_none(), host.count(4, "plank")),
        (true, 2),
        "so is an offer that holds more than the area takes"
    );
    host.carry(4, &[host.planks(1)]);
    host.choose(4, EntranceAnswer::GoAlone);
    assert_eq!(
        (host.run_of(4).is_none(), host.count(4, "plank")),
        (true, 1),
        "and an offer the bag doesn't cover"
    );
    host.carry(4, &[host.planks(2)]);
    host.choose(4, EntranceAnswer::GoAlone);
    let his = host
        .run_of(4)
        .expect("the right items let a player in alone where the area may be done alone");
    assert_eq!(
        (host.count(4, "plank"), host.entrance(4)),
        (0, None),
        "what it takes is taken as the instance opens"
    );
    let village = host.server.village.id::<AreaDef>("village");
    assert_eq!(
        (his.return_to, host.entrance(4)),
        (
            Place {
                area: village,
                at: BY_THE_DOOR
            },
            None
        )
    );
    assert!(
        host.at(4).distance(ENTRANCE) < Meters(1.5),
        "he appears where the door leads"
    );
    let osric = host.sim(4).player(4);
    assert_eq!(
        host.sim(4)
            .world
            .get::<SpawnPoint>(osric)
            .map(|point| point.0),
        Some(ENTRANCE),
        "whoever falls inside rises at the entrance"
    );
    let plate = host.shown(4).expect("a plate inside");
    assert_eq!(
        (goals(&plate), roster(&plate)),
        (
            vec![
                ("Defeat the warden".to_owned(), 0, 2),
                ("Seal the gate".to_owned(), 0, 1)
            ],
            vec![("Osric".to_owned(), RosterPlace::Inside)]
        ),
        "the plate says what the area asks of those inside, and who it opened for"
    );
    let below = Place {
        area: host.server.village.id::<AreaDef>("vault"),
        at: ENTRANCE,
    };
    let tamsin = host.sim(5).player(5);
    assert!(
        area::warp::warp(&mut host.sim(5).world, tamsin, below).is_err(),
        "nothing else leads into an instanced area"
    );
    uses(&mut host, 4, "compass");
    uses(&mut host, 5, "compass");
    assert_eq!(
        (host.count(4, "pebble"), host.count(5, "pebble")),
        (1, 0),
        "a script reads whether someone is in an instance"
    );

    uses(&mut host, 5, "archive_key");
    let panel = host
        .entrance(5)
        .expect("a panel at an area that needs more players");
    assert_eq!(
        (panel.alone_opens(), panel.party),
        (false, EntranceParty::Alone),
        "a player in no party can't go alone where the area needs more players"
    );
    host.pay_and_choose(5, EntranceAnswer::GoAlone);
    assert_eq!(
        (
            host.run_of(5),
            host.count(5, "pebble"),
            host.count(5, "plank")
        ),
        (None, 1, 3),
        "nor does choosing it anyway let them in: the door takes its refusal, and nothing"
    );

    party(&mut host, 1, &[(2, "Hale"), (3, "Marrow")]);
    uses(&mut host, 1, "vault_key");
    let panel = host.entrance(1).expect("a panel for a party's leader");
    assert_eq!(
        (panel.party, host.run_of(1)),
        (
            EntranceParty::Leads {
                size: 3,
                nearby: vec!["Hale".to_owned(), "Marrow".to_owned()]
            },
            None
        ),
        "a leader is offered the party, counting who stands near"
    );
    host.carry(1, &[host.planks(2)]);
    host.leave(1);
    assert_eq!(
        (
            host.entrance(1),
            host.run_of(1),
            host.count(1, "plank"),
            host.count(1, "pebble")
        ),
        (None, None, 2, 0),
        "leaving the panel lets nobody in and takes nothing"
    );
    host.at_door(1);
    host.door(1);
    assert!(host.entrance(1).is_some(), "the door opens the panel too");
    let wren = host.sim(1).player(1);
    movement::relocate(&mut host.sim(1).world, wren, Pos::new(2.0, 2.0));
    run(&mut host, 0.5);
    assert_eq!(
        (host.entrance(1), host.count(1, "pebble")),
        (None, 1),
        "walking away from the door closes its panel, and the door takes its refusal"
    );
    host.at_door(1);
    host.door(1);
    assert!(host.entrance(1).is_some(), "so the door may be used again");
    host.leave(1);
    assert_eq!(
        (host.entrance(1), host.count(1, "pebble")),
        (None, 2),
        "and leaving it gives the door's script its refusal"
    );

    uses(&mut host, 2, "vault_key");
    assert_eq!(
        host.entrance(2).map(|panel| panel.party),
        Some(EntranceParty::Member {
            leader: "Wren".to_owned()
        }),
        "a member is told who takes the party in"
    );
    host.pay_and_choose(2, EntranceAnswer::Gather);
    assert_eq!(
        (host.run_of(2), host.call(1), host.count(2, "plank")),
        (None, None, 2),
        "only the leader gathers the party"
    );
    uses(&mut host, 2, "vault_key");
    host.choose(2, EntranceAnswer::GoAlone);
    let hers = host.run_of(2).expect("a member goes alone");
    assert_ne!(hers.id, his.id);
    assert_eq!(
        host.shown(2).map(|plate| roster(&plate)),
        Some(vec![("Hale".to_owned(), RosterPlace::Inside)]),
        "going alone takes nobody else of the party"
    );

    uses(&mut host, 3, "vault_key");
    assert_eq!(
        host.entrance(3).map(|panel| panel.party),
        Some(EntranceParty::Inside),
        "a member whose party is inside is told so"
    );
    host.pay_and_choose(3, EntranceAnswer::GoAlone);
    assert_eq!(
        (host.run_of(3), host.errors(3), host.count(3, "plank")),
        (None, vec!["There is no room right now".to_owned()], 2),
        "the server runs only as many as it is set to, and a start refused takes nothing"
    );
    uses(&mut host, 5, "token");
    host.choose(5, EntranceAnswer::GoAlone);
    assert_eq!(
        (host.count(5, "pebble"), host.errors(5)),
        (2, Vec::<String>::new()),
        "a door that has its own say of a refusal says it alone"
    );
}

#[test]
fn a_leader_gathers_who_joins_within_sight_and_picks_who_goes() {
    let mut host = Host::of(game(3), 4);
    let names = ["Wren", "Hale", "Marrow", "Isolde", "Corvin"];
    for (client, name) in (1..).zip(names) {
        host.sign_in(client, name);
    }
    party(
        &mut host,
        1,
        &[(2, "Hale"), (3, "Marrow"), (4, "Isolde"), (5, "Corvin")],
    );
    host.go_to_forest(5);
    gather(&mut host, 1);

    let ends = host.call(1).expect("a call").ends;
    assert_eq!(
        rows(&host.call(1).expect("the roster")),
        [
            ("Wren", RosterRowShown::Leading),
            ("Hale", RosterRowShown::Deciding { ends }),
            ("Marrow", RosterRowShown::Deciding { ends }),
            ("Isolde", RosterRowShown::Deciding { ends }),
            ("Corvin", too_far(&host)),
        ],
        "the roster is the party, each member as they stand toward the leader"
    );
    assert_eq!(
        (2..=5)
            .map(|client| host.call(client).is_some())
            .collect::<Vec<_>>(),
        [true, true, true, false],
        "every member in sight is called, and nobody out of it"
    );

    host.reply(2, CallReply::Join);
    host.reply(3, CallReply::Join);
    host.reply(4, CallReply::Join);
    let call = host.call(1).expect("the roster");
    assert_eq!(
        (states(&call)[1..4].to_vec(), call.taken, call.ready),
        (
            vec![
                RosterRowShown::Going,
                RosterRowShown::Going,
                RosterRowShown::Benched
            ],
            3,
            3
        ),
        "who joins is picked in the order they answered, while a slot is free"
    );
    assert_eq!(
        host.call(4).map(|card| states(&card)),
        Some(vec![RosterRowShown::Benched]),
        "a member's card shows where they stand"
    );

    host.reply(3, CallReply::Decline);
    let call = host.call(1).expect("the roster");
    assert_eq!(
        (states(&call)[2..4].to_vec(), call.taken),
        (vec![RosterRowShown::NotGoing, RosterRowShown::Benched], 2),
        "a member may change their mind, and a slot that frees stays free for the leader"
    );
    host.reply(3, CallReply::Join);
    assert_eq!(
        states(&host.call(1).expect("the roster"))[2],
        RosterRowShown::Going,
        "and change it back"
    );

    host.ask_call(1, CallRequest::Pick(host.server.number("Isolde")));
    assert_eq!(
        states(&host.call(1).expect("the roster"))[3],
        RosterRowShown::Benched,
        "nobody is picked while every slot is taken"
    );
    host.ask_call(1, CallRequest::Unpick(host.server.number("Marrow")));
    host.ask_call(1, CallRequest::Pick(host.server.number("Isolde")));
    assert_eq!(
        states(&host.call(1).expect("the roster"))[2..4],
        [RosterRowShown::Benched, RosterRowShown::Going],
        "the leader swaps who goes"
    );

    let hale = host.sim(2).player(2);
    combat::inflict(&mut host.sim(2).world, hale, None, 1_000_000.0);
    run(&mut host, 0.5);
    let call = host.call(1).expect("the roster");
    assert_eq!(
        (states(&call)[1], call.ready),
        (RosterRowShown::Fallen, 2),
        "a fallen member is not counted"
    );
    stat::revive(&mut host.sim(2).world, hale);
    run(&mut host, 0.5);
    host.go_to_forest(4);
    let call = host.call(1).expect("the roster");
    assert_eq!(
        (
            states(&call)[3],
            call.ready,
            host.call(4).map(|card| states(&card))
        ),
        (too_far(&host), 2, Some(vec![too_far(&host)])),
        "one who walks out of sight keeps their answer, and is not counted"
    );

    let wren = host.sim(1).player(1);
    combat::inflict(&mut host.sim(1).world, wren, None, 1_000_000.0);
    run(&mut host, 0.5);
    host.ask_call(1, CallRequest::Enter);
    let call = host.call(1).expect("the roster");
    assert_eq!(
        (states(&call)[0], call.ready, host.run_of(1), host.run_of(2)),
        (RosterRowShown::Fallen, 0, None, None),
        "a fallen leader takes nobody in"
    );
    stat::revive(&mut host.sim(1).world, wren);
    run(&mut host, 0.5);

    host.ask_call(1, CallRequest::Unpick(host.server.number("Isolde")));
    host.ask_call(1, CallRequest::Pick(host.server.number("Marrow")));
    let before = [3, 4, 5].map(|client| host.told(client).len());
    host.send(3, CallReply::Decline);
    host.carry(2, &[host.planks(2)]);
    assert_eq!(host.count(1, "plank"), 2, "gathering takes nothing");
    host.send(1, CallRequest::Enter);
    run(&mut host, 0.5);
    let inside = host.run_of(1).expect("the leader goes in on Enter");
    assert_eq!(
        (host.count(1, "plank"), host.count(2, "plank")),
        (0, 2),
        "only the leader pays, as the instance opens"
    );
    assert_eq!(
        (
            host.run_of(2),
            host.run_of(3),
            host.run_of(4),
            host.run_of(5)
        ),
        (Some(inside), None, None, None),
        "the leader takes everyone picked who joined and stands within sight, as Enter finds them"
    );
    assert_eq!(
        host.shown(1).map(|plate| roster(&plate)),
        Some(vec![
            ("Wren".to_owned(), RosterPlace::Inside),
            ("Hale".to_owned(), RosterPlace::Inside)
        ]),
        "a withdrawal that came before Enter takes its member off the roster"
    );
    assert_eq!(
        [3, 4, 5]
            .into_iter()
            .zip(before)
            .map(|(client, before)| host.told(client)[before..].to_vec())
            .collect::<Vec<_>>(),
        [
            Vec::new(),
            vec!["The party went into vault without you. You were too far from Wren.".to_owned()],
            Vec::new(),
        ],
        "who joined and was not taken is told why; who didn't join is told nothing"
    );
    assert_eq!(
        (host.call(1), host.call(3), host.count(1, "pebble")),
        (None, None, 0),
        "entering ends the call, and the door's script carries on"
    );
}

#[test]
fn a_call_ends_with_its_leaders_lead_their_place_at_the_door_or_its_time() {
    let mut host = Host::of(game(3), 4);
    for (client, name) in [(1, "Wren"), (2, "Hale"), (3, "Marrow")] {
        host.sign_in(client, name);
    }
    party(&mut host, 1, &[(2, "Hale"), (3, "Marrow")]);

    uses(&mut host, 1, "archive_key");
    host.pay_and_choose(1, EntranceAnswer::Gather);
    host.ask_call(1, CallRequest::Enter);
    assert_eq!(
        (
            host.run_of(1),
            host.errors(1).last().cloned(),
            host.call(1).is_some()
        ),
        (
            None,
            Some("archive needs at least 2 players.".to_owned()),
            true
        ),
        "Enter waits until enough would go"
    );
    host.ask_call(1, CallRequest::CallOff);
    assert_eq!(
        (
            host.call(1),
            host.call(2),
            host.told(2).last().cloned(),
            host.count(1, "pebble")
        ),
        (None, None, Some("Wren called off archive.".to_owned()), 1),
        "calling it off tells everyone called, and the door takes its refusal"
    );

    let ended: [(&str, Ending); 4] = [
        ("The call to vault ran out of time.", |host, _| {
            host.later(121.0)
        }),
        (
            "Wren no longer leads the party. The call to vault ended.",
            |host, leader| {
                let next = host.server.number("Hale");
                host.send(leader, PartyRequest::Lead(next));
                run(host, 0.5);
            },
        ),
        (
            "Wren walked away from the entrance. The call ended.",
            |host, leader| {
                let character = host.sim(leader).player(leader);
                movement::relocate(&mut host.sim(leader).world, character, Pos::new(2.0, 2.0));
                run(host, 0.5);
            },
        ),
        (
            "Wren no longer leads the party. The call to vault ended.",
            |host, leader| {
                host.sim(leader).disconnect(leader);
                run(host, 0.5);
            },
        ),
    ];
    for (index, (said, end)) in ended.into_iter().enumerate() {
        let leader = host.leader();
        let member = if leader == 2 { 3 } else { 2 };
        host.at_door(leader);
        host.door(leader);
        host.pay_and_choose(leader, EntranceAnswer::Gather);
        assert!(host.call(member).is_some(), "a call opens at the door");
        let refused = host.count(leader, "pebble");
        end(&mut host, leader);
        let said = said.replace("Wren", &host.name(leader));
        assert_eq!(
            (
                host.call(member),
                host.told(member).last().cloned(),
                host.society_call(leader)
            ),
            (None, Some(said.clone()), false),
            "{said}: everyone called is told"
        );
        let gone = index == 3;
        if !gone {
            assert_eq!(
                host.count(leader, "pebble"),
                refused + 1,
                "{said}: the door takes its refusal"
            );
        }
    }
}

#[test]
fn a_copy_belongs_to_the_roster_it_opened_with() {
    let mut host = Host::of(game(4), 4);
    let names = ["Wren", "Hale", "Marrow", "Isolde", "Corvin"];
    for (client, name) in (1..).zip(names) {
        host.sign_in(client, name);
    }
    party(
        &mut host,
        1,
        &[(2, "Hale"), (3, "Marrow"), (4, "Isolde"), (5, "Corvin")],
    );
    gather(&mut host, 1);
    for client in 2..=4 {
        host.reply(client, CallReply::Join);
    }
    host.ask_call(1, CallRequest::Enter);
    let ours = host.run_of(1).expect("the party went in");

    uses(&mut host, 5, "vault_key");
    assert_eq!(
        host.entrance(5).map(|panel| panel.party),
        Some(EntranceParty::Inside),
        "nobody joins an instance once it opened"
    );
    host.pay_and_choose(5, EntranceAnswer::GoAlone);
    let his = host.run_of(5).expect("an instance of his own");
    assert_ne!(his.id, ours.id);
    host.send(5, LeaveInstance);
    run(&mut host, 0.5);

    host.send(3, LeaveInstance);
    run(&mut host, 0.5);
    assert_eq!(
        (
            host.run_of(3),
            host.at(3).distance(BY_THE_DOOR) < Meters(1.5)
        ),
        (None, true),
        "leaving leads back to the door"
    );
    assert_eq!(
        (
            host.shown(1).map(|plate| roster(&plate)[2].clone()),
            host.told(1).last().cloned()
        ),
        (
            Some(("Marrow".to_owned(), RosterPlace::Left)),
            Some("Marrow left vault.".to_owned())
        ),
        "whoever leaves is listed as left, and the others are told"
    );
    uses(&mut host, 3, "vault_key");
    host.pay_and_choose(3, EntranceAnswer::GoAlone);
    assert_ne!(
        host.run_of(3).map(|run| run.id),
        Some(ours.id),
        "leaving is for good"
    );
    uses(&mut host, 3, "rope");

    let removed = host.server.number("Hale");
    host.send(1, PartyRequest::Remove(removed));
    run(&mut host, 1.0);
    host.later(60.0);
    assert_eq!(
        (
            host.run_of(2),
            host.shown(1).map(|plate| roster(&plate)[1].1)
        ),
        (Some(ours), Some(RosterPlace::Inside)),
        "party changes don't reach inside an instance"
    );

    let stood = host.at(4);
    host.sim(4).disconnect(4);
    run(&mut host, 0.5);
    assert_eq!(
        host.shown(1).map(|plate| roster(&plate)[3].1),
        Some(RosterPlace::Away),
        "a member whose player is gone is away, their place kept"
    );
    host.later(31.0);
    let saved = host.saved(4);
    assert_eq!(
        saved.character.start,
        CharacterStart::At(Place {
            area: host.server.village.id::<AreaDef>("village"),
            at: BY_THE_DOOR
        }),
        "a character saved inside is saved where leaving leads"
    );
    host.come_back(4, "Isolde", &saved);
    assert_eq!(
        (
            host.run_of(4),
            host.at(4).distance(stood) < Meters(1.0),
            host.shown(1).map(|plate| roster(&plate)[3].1)
        ),
        (Some(ours), true, Some(RosterPlace::Inside)),
        "a member who comes back while the instance lives is back inside, where they stood"
    );
    let isolde = host.sim(4).player(4);
    assert_eq!(
        host.sim(4)
            .world
            .get::<SpawnPoint>(isolde)
            .map(|point| point.0),
        Some(ENTRANCE),
        "and rises at its entrance"
    );

    host.sim(4).disconnect(4);
    host.later(31.0);
    let saved = host.saved(4);
    let content = host.content.clone();
    let stack = |name: &str, count: u32| {
        let item = content
            .by_path::<ItemDef>(&path("item", name))
            .expect("an item of the test game");
        ItemStack::new(item, count)
    };
    let spoils = vec![
        stack("potion", 2),
        stack("gold", 30),
        stack("warden_key", 1),
    ];
    clear_vault(&mut host, 1);
    assert_eq!(
        [1, 2, 3].map(|client| (host.bag(client), host.xp(client))),
        [(spoils.clone(), 50), (spoils.clone(), 50), (Vec::new(), 0)],
        "clearing rewards whoever is inside, and not who left"
    );
    host.later(31.0);
    assert_eq!(host.instances(), 0, "a cleared instance closes");
    host.come_back(4, "Isolde", &saved);
    assert_eq!(
        (
            host.run_of(4),
            host.at(4).distance(BY_THE_DOOR) < Meters(1.5),
            host.told(4).last().cloned(),
            sent(&mut host, 4)
        ),
        (
            None,
            true,
            Some("vault ended while you were away. You're back in village.".to_owned()),
            vec![("Your rewards from vault".to_owned(), true, spoils)]
        ),
        "a member away when it ended comes back where leaving leads, told why, their rewards \
         sent as a message"
    );
}

#[test]
fn clearing_a_copy_rewards_each_of_its_roster_once() {
    let mut host = Host::of(game(5), 2);
    let names = ["Wren", "Hale", "Isolde", "Tamsin", "Corvin"];
    for (client, name) in (1..).zip(names) {
        host.sign_in(client, name);
    }
    party(
        &mut host,
        1,
        &[(2, "Hale"), (3, "Isolde"), (4, "Tamsin"), (5, "Corvin")],
    );
    gather(&mut host, 1);
    for client in 2..=5 {
        host.reply(client, CallReply::Join);
    }
    host.ask_call(1, CallRequest::Enter);
    let content = host.content.clone();
    let stack = |name: &str, count: u32| {
        let item = content
            .by_path::<ItemDef>(&path("item", name))
            .expect("an item of the test game");
        ItemStack::new(item, count)
    };
    let spoils = [
        stack("potion", 2),
        stack("gold", 30),
        stack("warden_key", 1),
    ];

    host.carry(
        1,
        &[stack("gold", 5), stack("plank", 50), stack("plank", 50)],
    );
    let wren = host.sim(1).player(1);
    for fallen in 1..=2 {
        let warden = warden(&mut host, 1);
        combat::inflict(&mut host.sim(1).world, warden, Some(wren), 1_000_000.0);
        run(&mut host, 0.5);
        let plate = host.shown(2).expect("a plate inside");
        assert_eq!(
            goals(&plate)[0],
            ("Defeat the warden".to_owned(), fallen, 2),
            "a goal is counted for everyone inside, whoever struck the blow"
        );
    }
    let corvin = host.sim(5).player(5);
    combat::inflict(&mut host.sim(5).world, corvin, None, 1_000_000.0);
    run(&mut host, 0.5);
    assert!(stat::is_dead(&host.sim(5).world, corvin));
    assert_eq!(host.xp(1), 0, "nothing is given before every goal is met");

    uses(&mut host, 2, "seal");
    assert_eq!(
        [1, 2, 5].map(|client| (host.bag(client), host.xp(client))),
        [
            (
                vec![
                    stack("gold", 35),
                    stack("plank", 50),
                    stack("plank", 50),
                    stack("potion", 2)
                ],
                50
            ),
            (spoils.to_vec(), 50),
            (spoils.to_vec(), 50),
        ],
        "each stack goes where it fits, to whoever was there, standing or fallen"
    );
    assert_eq!(
        (sent(&mut host, 1), sent(&mut host, 2)),
        (
            vec![(
                "Your rewards from vault".to_owned(),
                true,
                vec![stack("warden_key", 1)]
            )],
            Vec::new()
        ),
        "only what a bag had no room for is sent, by the game"
    );
    let cleared = host.shown(1).and_then(|plate| plate.cleared);
    assert_eq!(
        cleared.map(|cleared| cleared.given),
        Some(AreaInstanceGiven {
            xp: 50,
            kept: vec![stack("potion", 2), stack("gold", 30)],
            sent: vec![stack("warden_key", 1)],
        }),
        "each participant is shown what clearing it gave them"
    );

    uses(&mut host, 2, "seal");
    let warden = warden(&mut host, 1);
    combat::inflict(&mut host.sim(1).world, warden, Some(wren), 1_000_000.0);
    run(&mut host, 0.5);
    assert_eq!(
        (host.bag(2), host.xp(2), sent(&mut host, 1).len()),
        (spoils.to_vec(), 50, 1),
        "an instance is cleared once"
    );

    host.later(31.0);
    run(&mut host, 0.5);
    assert_eq!(
        [1, 2, 5].map(|client| (
            host.run_of(client),
            host.at(client).distance(BY_THE_DOOR) < Meters(2.5)
        )),
        [(None, true); 3],
        "a cleared instance closes, and everyone inside is returned"
    );
    assert_eq!(host.instances(), 0);
}

#[test]
fn a_copy_lives_while_its_roster_may_come_back_and_its_time_lasts() {
    let mut host = Host::of(game(3), 2);
    host.sign_in(1, "Wren");
    host.sign_in(4, "Osric");
    uses(&mut host, 1, "vault_key");
    host.pay_and_choose(1, EntranceAnswer::GoAlone);
    let first = host.run_of(1).expect("an instance");
    uses(&mut host, 1, "rope");
    assert_eq!(host.run_of(1), None, "a script leads out as the plate does");
    assert_eq!(
        host.instances(),
        0,
        "an instance everyone on its roster left is gone at once"
    );
    uses(&mut host, 1, "vault_key");
    let panel = host
        .entrance(1)
        .expect("the panel opens while the starter waits");
    let now = host.server.village.world.resource::<WallClock>().now;
    assert!(
        panel
            .ready_at
            .is_some_and(|ready| ready > now && ready <= now.after(Seconds(60.0))),
        "the panel shows when the starter may start another instance"
    );
    host.pay_and_choose(1, EntranceAnswer::GoAlone);
    assert_eq!(
        (host.run_of(1), host.count(1, "plank"), host.errors(1).len()),
        (None, 2, 1),
        "the starter is refused while they wait, and nothing is taken"
    );
    host.leave(1);
    host.later(61.0);
    uses(&mut host, 1, "vault_key");
    host.choose(1, EntranceAnswer::GoAlone);
    let second = host
        .run_of(1)
        .expect("another instance once the cooldown is out");
    assert_ne!(second.id, first.id);

    host.sim(1).disconnect(1);
    host.later(31.0);
    let saved = host.saved(1);
    host.later(301.0);
    assert_eq!(
        host.instances(),
        0,
        "one nobody came back to in the area's time is gone"
    );
    host.come_back(1, "Wren", &saved);
    assert_eq!(host.run_of(1), None);

    uses(&mut host, 4, "trial_key");
    host.pay_and_choose(4, EntranceAnswer::GoAlone);
    assert!(host.run_of(4).is_some());
    host.later(601.0);
    run(&mut host, 0.5);
    assert_eq!(
        (
            host.told(4).last().map(String::as_str),
            host.run_of(4),
            host.xp(4),
            host.instances()
        ),
        (Some("Time ran out in trial"), None, 0, 0),
        "an instance that only lasts so long returns everyone, with no rewards"
    );
}

#[test]
fn a_game_whose_instances_cannot_be_played_is_refused() {
    let rules = |game: Game| -> Vec<&'static str> {
        let report = game.try_load().err().expect("refused");
        report
            .errors
            .iter()
            .map(|error| error.problem.rule)
            .collect()
    };
    let door = |change: &dyn Fn(&mut Value)| {
        game(2).edit("script", "vault_door", |script| {
            change(&mut script["nodes"]["enter"])
        })
    };
    let place = |area: &str| json!({ "area": path("area", area), "x": 3.0, "y": 3.0 });

    assert_eq!(
        rules(door(
            &|enter| *enter = json!({ "type": "warp", "to": place("vault") })
        )),
        ["warps into an instance"]
    );
    assert_eq!(
        rules(door(&|enter| enter["to"] = place("forest"))),
        ["not an instanced area"]
    );
    assert_eq!(
        rules(door(&|enter| enter["return_to"] = place("trial"))),
        ["returns into an instance"]
    );
    assert_eq!(
        rules(game(2).edit_settings(|settings| {
            settings["initial_spawn"]["area"] = json!(path("area", "vault"))
        })),
        ["starts in an instance"]
    );
    let scout = json!({
        "name": "Scout",
        "class": path("class", "hero"),
        "appearance": { "model": path("model", "hero") },
        "start": place("vault"),
    });
    assert_eq!(
        rules(game(2).row("test_character", "scout", scout)),
        ["starts in an instance"]
    );
    let mut unsent = rules(game(2).edit_settings(|settings| {
        settings.as_object_mut().expect("settings").remove("mail");
    }));
    unsent.dedup();
    assert_eq!(
        unsent,
        ["delivers items without mail"],
        "what a bag has no room for needs somewhere to go"
    );
    assert_eq!(
        rules(game(2).edit("area", "vault", |vault| {
            vault["instance"]["goals"][0] = vault["instance"]["goals"][1].clone()
        })),
        ["task named twice"]
    );
    assert_eq!(
        rules(game(2).edit("area", "vault", |vault| {
            vault["instance"]["players"] = json!({ "least": 3, "most": 2 })
        })),
        ["out of range"],
        "an instance can't need more players than it takes"
    );
}

/// What ends a call, done to the server of the call's leader, the player of the client given.
type Ending = fn(&mut Host, u32);

/// A server whose instances each get a world of their own as someone is let in, the way a
/// game server makes them.
struct Host {
    server: Server,
    runs: Vec<Sim>,
    content: Content,
    /// The account each client signed in as.
    names: std::collections::BTreeMap<u32, String>,
    /// What the feed told each player in the worlds they have left, by their client.
    told_before: Vec<(u32, String)>,
}

impl Host {
    /// A server of `game` that runs at most `instances` instances at once.
    fn of(game: Game, instances: u32) -> Host {
        let mut server = Server::hosting(game, request_sources(), instances);
        plays(&mut server.village);
        plays(&mut server.forest);
        let content = server.village.world.resource::<Content>().clone();
        Host {
            server,
            runs: Vec::new(),
            content,
            names: std::collections::BTreeMap::new(),
            told_before: Vec::new(),
        }
    }

    fn planks(&self, count: u32) -> ItemStack {
        let item = self
            .content
            .by_path::<ItemDef>(&path("item", "plank"))
            .expect("an item of the test game");
        ItemStack::new(item, count)
    }

    fn sims(&mut self) -> Vec<&mut Sim> {
        [&mut self.server.village, &mut self.server.forest]
            .into_iter()
            .chain(&mut self.runs)
            .collect()
    }

    /// The worlds confer, each plays a tick, and whoever is on their way arrives.
    fn step(&mut self) {
        let Server {
            above,
            village,
            forest,
        } = &mut self.server;
        let mut sims: Vec<&mut Sim> = [village, forest]
            .into_iter()
            .chain(&mut self.runs)
            .collect();
        confer(&mut sims, above);
        for sim in sims {
            sim.tick();
        }
        self.travel();
        let society = self.server.above.society.read();
        self.runs.retain(|sim| {
            let run = sim.world.resource::<AreaInstanceWorld>().id;
            society.instances().get(run).is_some()
        });
    }

    fn travel(&mut self) {
        let mut travelers = Vec::new();
        let mut told = Vec::new();
        for sim in self.sims() {
            for traveler in transition::departing(&mut sim.world) {
                let client = traveler.played.client.0;
                let connected = connection(sim, client).is_some();
                if connected {
                    told.extend(told_in(sim, client).into_iter().map(|said| (client, said)));
                    sim.disconnect(client);
                }
                travelers.push((traveler, connected));
            }
        }
        self.told_before.extend(told);
        for (traveler, connected) in travelers {
            let client = traveler.played.client.0;
            let to = self.world_for(traveler.dest_area, traveler.instance);
            if connected {
                to.connect(client);
            }
            transition::arrive(&mut to.world, traveler);
            if connected {
                let conn = to.conn(client);
                to.send(conn, AreaDrawn);
            }
        }
    }

    /// The world of `area` everyone shares, or of its instance `run`, made now if it wasn't.
    fn world_for(&mut self, area: AreaId, run: Option<AreaInstanceWorld>) -> &mut Sim {
        let Some(run) = run else {
            let village = self.server.village.id::<AreaDef>("village");
            return if area == village {
                &mut self.server.village
            } else {
                &mut self.server.forest
            };
        };
        let made = self
            .runs
            .iter()
            .position(|sim| sim.world.resource::<AreaInstanceWorld>().id == run.id);
        let index = made.unwrap_or_else(|| {
            let key = ["vault", "trial", "archive"]
                .into_iter()
                .find(|key| self.server.village.id::<AreaDef>(key) == area)
                .expect("an instanced area of the test game");
            let mut sim = serving(&self.content, key);
            plays(&mut sim);
            sim.world.insert_resource(run);
            let now = *self.server.village.world.resource::<WallClock>();
            sim.world.insert_resource(now);
            self.runs.push(sim);
            self.runs.len() - 1
        });
        &mut self.runs[index]
    }

    fn later(&mut self, seconds: f32) {
        for sim in self.sims() {
            let mut clock = sim.world.resource_mut::<WallClock>();
            clock.now = clock.now.after(Seconds(seconds));
        }
        self.step();
        self.step();
    }

    /// The account called `name` signs in and joins the village as `client`.
    fn sign_in(&mut self, client: u32, name: &str) -> Entity {
        self.names.insert(client, name.to_owned());
        let character = self.server.sign_in(client, name, "village");
        self.step();
        character
    }

    /// The character `saved` of the account called `name` signs in again as `client`, where the
    /// server lets it in: back inside an instance it was away from while that lives, else where it
    /// was saved.
    fn come_back(&mut self, client: u32, name: &str, saved: &SavedCharacter) {
        self.names.insert(client, name.to_owned());
        let account = engine::systems::account::AccountId(name.to_owned());
        let rejoin = self
            .server
            .above
            .society
            .read()
            .instances()
            .rejoin(&account);
        let start = match &saved.character.start {
            CharacterStart::At(place) => *place,
            CharacterStart::InitialSpawn(_) => panic!("a character saved somewhere"),
        };
        let (run, start) = match rejoin {
            Some((run, place)) => (Some(run), place),
            None => (None, start),
        };
        let join_as = JoinAs {
            snapshot: saved.character.clone(),
            start,
            origin: JoinOrigin::Saved {
                at: saved.saved_at,
                told: Vec::new(),
            },
        };
        let sim = self.world_for(start.area, run);
        sim.sign_in(client, name, Some(join_as));
        run_steps(self, 10);
    }

    /// What was saved of the character of `client` as it left its world, once it has.
    fn saved(&mut self, client: u32) -> SavedCharacter {
        let account = engine::systems::account::AccountId(self.name(client));
        self.server
            .above
            .checkpoints
            .departed(&account)
            .map(|saved| (*saved).clone())
            .expect("a character that left its world")
    }

    /// The account `client` signed in as.
    fn name(&self, client: u32) -> String {
        self.names
            .get(&client)
            .cloned()
            .expect("a client that signed in")
    }

    /// The world the character of `client` stands in.
    fn sim(&mut self, client: u32) -> &mut Sim {
        self.sims()
            .into_iter()
            .find(|sim| {
                sim.world
                    .resource::<Players>()
                    .0
                    .contains_key(&ClientId(client))
            })
            .expect("a character in a world")
    }

    fn send<M: Message>(&mut self, client: u32, message: M) {
        let sim = self.sim(client);
        let conn = sim.conn(client);
        sim.send(conn, message);
        sim.tick();
    }

    /// The instance the character of `client` stands in.
    fn run_of(&mut self, client: u32) -> Option<AreaInstanceWorld> {
        self.sim(client)
            .world
            .get_resource::<AreaInstanceWorld>()
            .copied()
    }

    fn at(&mut self, client: u32) -> Pos<Meters> {
        let sim = self.sim(client);
        position(&sim.world, sim.player(client)).expect("a character somewhere")
    }

    /// The player of `client` walks over to the forest.
    fn go_to_forest(&mut self, client: u32) {
        let forest = self.server.village.id::<AreaDef>("forest");
        let sim = self.sim(client);
        let character = sim.player(client);
        let to = Place {
            area: forest,
            at: Pos::new(6.0, 6.0),
        };
        area::warp::cross(&mut sim.world, character, to, None);
        run(self, 0.5);
    }

    /// The character of `client` stands beside the vault's door.
    fn at_door(&mut self, client: u32) {
        let sim = self.sim(client);
        let character = sim.player(client);
        movement::relocate(&mut sim.world, character, Pos::new(DOOR.x + 1.0, DOOR.y));
        run(self, 0.2);
    }

    /// The player of `client` uses the vault's door and chooses to go down, as its conversation
    /// asks before the entrance panel opens.
    fn door(&mut self, client: u32) {
        let target = self.sim(client).npc("door");
        self.send(client, InteractRequest { target });
        run(self, 0.5);
        self.sim(client).pick(client, "Go down.");
        run(self, 0.5);
    }

    /// The entrance panel the player of `client` is shown.
    fn entrance(&mut self, client: u32) -> Option<EntranceView> {
        let sim = self.sim(client);
        let character = sim.player(client);
        sim.world.get::<EntranceView>(character).cloned()
    }

    /// The player of `client` inserts `offer` at their entrance panel and chooses `answer`.
    fn choose_with(&mut self, client: u32, answer: EntranceAnswer, offer: Vec<ItemStack>) {
        let step = self.entrance(client).expect("an entrance panel").step;
        self.send(
            client,
            EntranceChoice {
                step,
                answer,
                offer,
            },
        );
        run(self, 0.5);
    }

    /// The player of `client` inserts what their entrance panel's area takes and chooses `answer`.
    fn choose(&mut self, client: u32, answer: EntranceAnswer) {
        let offer = self.fee(client);
        self.choose_with(client, answer, offer);
    }

    /// The player of `client` is handed what their entrance panel's area takes, and chooses
    /// `answer` with it.
    fn pay_and_choose(&mut self, client: u32, answer: EntranceAnswer) {
        let offer = self.fee(client);
        let sim = self.sim(client);
        let character = sim.player(client);
        sim.give(
            character,
            "plank",
            offer.iter().map(|stack| stack.count).sum(),
        );
        self.choose_with(client, answer, offer);
    }

    /// What the area of the entrance panel of `client` takes.
    fn fee(&mut self, client: u32) -> Vec<ItemStack> {
        let area = self.entrance(client).expect("an entrance panel").area;
        area.get(&self.content)
            .instance
            .as_ref()
            .expect("an instanced area")
            .entry
            .needs
            .clone()
    }

    /// The player of `client` leaves the conversation their entrance panel is in.
    fn leave(&mut self, client: u32) {
        self.sim(client).leave(client);
        run(self, 0.5);
    }

    /// The call the player of `client` leads, or was called to.
    fn call(&mut self, client: u32) -> Option<InstanceCallShown> {
        let sim = self.sim(client);
        let character = sim.player(client);
        sim.world.get::<SocietyView>(character)?.call.clone()
    }

    /// Whether society holds a call `client`'s player leads.
    fn society_call(&mut self, client: u32) -> bool {
        let account = engine::systems::account::AccountId(self.name(client));
        self.server
            .above
            .society
            .read()
            .instances()
            .calls()
            .led_by(&account)
            .is_some()
    }

    fn ask_call(&mut self, client: u32, request: CallRequest) {
        self.send(client, request);
        run(self, 0.5);
    }

    fn reply(&mut self, client: u32, reply: CallReply) {
        self.send(client, reply);
        run(self, 0.5);
    }

    /// The client of the party's leader.
    fn leader(&self) -> u32 {
        let society = self.server.above.society.read();
        let any = engine::systems::account::AccountId(self.name(1));
        let leader = &society.parties().of_account(&any).expect("a party").leader;
        self.names
            .iter()
            .find(|(_, name)| leader.0 == **name)
            .map(|(client, _)| *client)
            .expect("a leader who signed in")
    }

    /// The instance the player of `client` is shown they are inside.
    fn shown(&mut self, client: u32) -> Option<AreaInstanceShown> {
        let sim = self.sim(client);
        let character = sim.player(client);
        sim.world.get::<SocietyView>(character)?.instance.clone()
    }

    fn inbox(&mut self, client: u32) -> InboxView {
        let sim = self.sim(client);
        let character = sim.player(client);
        sim.world
            .get::<InboxView>(character)
            .cloned()
            .unwrap_or_default()
    }

    fn bag(&mut self, client: u32) -> Vec<ItemStack> {
        let sim = self.sim(client);
        let character = sim.player(client);
        sim.world
            .get::<Inventory>(character)
            .expect("a bag")
            .stacks()
            .map(|(_, stack)| stack)
            .collect()
    }

    /// The character of `client` carries `stacks` and nothing else.
    fn carry(&mut self, client: u32, stacks: &[ItemStack]) {
        let sim = self.sim(client);
        let character = sim.player(client);
        let mut bag = sim.world.get_mut::<Inventory>(character).expect("a bag");
        let size = bag.slots.len();
        bag.slots = stacks
            .iter()
            .copied()
            .map(Some)
            .chain(std::iter::repeat(None))
            .take(size)
            .collect();
    }

    fn count(&mut self, client: u32, item: &str) -> u32 {
        let sim = self.sim(client);
        sim.count(sim.player(client), item)
    }

    fn xp(&mut self, client: u32) -> u32 {
        let sim = self.sim(client);
        let character = sim.player(client);
        sim.world.get::<Xp>(character).map_or(0, |xp| xp.amount)
    }

    fn errors(&mut self, client: u32) -> Vec<String> {
        self.sim(client).errors(client)
    }

    /// What the feed told the player so far, in every world they have been in, refusals aside.
    fn told(&mut self, client: u32) -> Vec<String> {
        let here = told_in(self.sim(client), client);
        let before = self.told_before.iter();
        before
            .filter(|(told, _)| *told == client)
            .map(|(_, said)| said.clone())
            .chain(here)
            .collect()
    }

    /// How many instances the server runs.
    fn instances(&self) -> usize {
        self.server.above.society.read().instances().iter().count()
    }
}

/// Has a world play what instances take: items used, doors, the scripts they run, falls, the
/// entrance panel, calls and leaving.
fn plays(sim: &mut Sim) {
    sim.accept::<AreaDrawn>()
        .accept::<UseItemRequest>()
        .accept::<LeaveInstance>()
        .accept::<InteractRequest>()
        .accept::<EntranceChoice>()
        .accept::<ConversationRequest>()
        .accept::<CallRequest>()
        .accept::<CallReply>()
        .add((
            player::arrival_holds,
            item::use_item,
            interact::interact_request,
            dialogue::requests,
            area_instance::requests,
            area_instance::entrance::requests,
            area_instance::roster::requests,
        ))
        .plan((
            engine::systems::roster::ListRoster::default(),
            engine::core::script::run::FlowRunner::default(),
            movement::AdvanceWalks::default(),
        ))
        .add(
            (
                interact::interactions,
                dialogue::hold,
                dialogue::refresh,
                window::hold::<prompt::PromptSession>,
                window::hold::<area_instance::entrance::EntranceSession>,
            )
                .chain(),
        )
        .plan(stat::SettleVitals::default())
        .add((combat::defeats, area_instance::count_defeats).chain());
    engine::systems::spawn_scripts(&mut sim.world);
}

/// What the feed told the player of `client` in this world, refusals aside.
fn told_in(sim: &mut Sim, client: u32) -> Vec<String> {
    let errors = sim.errors(client);
    sim.told(client)
        .into_iter()
        .map(|told| told.text.plain_words(sim.content()))
        .filter(|said| !errors.contains(said))
        .collect()
}

fn connection(sim: &mut Sim, client: u32) -> Option<Entity> {
    let mut conns = sim.world.query::<(Entity, &ClientId)>();
    conns
        .iter(&sim.world)
        .find(|(_, id)| id.0 == client)
        .map(|(conn, _)| conn)
}

fn run(host: &mut Host, seconds: f32) {
    let ticks = (seconds * engine::systems::TICK_HZ.0).ceil() as usize;
    run_steps(host, ticks);
}

fn run_steps(host: &mut Host, steps: usize) {
    for _ in 0..steps {
        host.step();
    }
}

/// The player of `leader` makes a party of themselves and every one of `members`.
fn party(host: &mut Host, leader: u32, members: &[(u32, &str)]) {
    for (client, name) in members {
        let invited = host.server.number(name);
        host.send(leader, PartyRequest::Invite(invited));
        host.step();
        let sim = host.sim(*client);
        let character = sim.player(*client);
        let request = sim
            .world
            .get::<SocietyView>(character)
            .expect("what the player sees of society")
            .requests[0]
            .id;
        host.send(
            *client,
            engine::systems::social_request::AnswerSocialRequest {
                request,
                answer: RequestAnswer::Accept,
            },
        );
        host.step();
    }
}

/// The party's leader, the player of `leader`, gathers the party at the vault's door.
fn gather(host: &mut Host, leader: u32) {
    host.at_door(leader);
    host.door(leader);
    host.pay_and_choose(leader, EntranceAnswer::Gather);
}

/// The player of `client` is handed `item`, uses it, and no longer has it once the world has
/// played what that started.
fn uses(host: &mut Host, client: u32, item: &str) {
    let sim = host.sim(client);
    let character = sim.player(client);
    sim.give(character, item, 1);
    let slot = sim.slot_of(character, item);
    host.send(client, UseItemRequest { slot });
    run(host, 0.5);
    let sim = host.sim(client);
    if sim.count(sim.player(client), item) > 0 {
        sim.take(sim.player(client), item, 1);
    }
}

/// Meets every goal of the vault the player of `client` is inside.
fn clear_vault(host: &mut Host, client: u32) {
    let character = host.sim(client).player(client);
    for _ in 0..2 {
        let warden = warden(host, client);
        combat::inflict(
            &mut host.sim(client).world,
            warden,
            Some(character),
            1_000_000.0,
        );
        run(host, 0.5);
    }
    uses(host, client, "seal");
}

/// A warden beside the character of `client`.
fn warden(host: &mut Host, client: u32) -> Entity {
    let at = host.at(client);
    let sim = host.sim(client);
    let (area, warden) = (sim.id::<AreaDef>("vault"), sim.id::<NpcDef>("warden"));
    npc::spawn(&mut sim.world, warden, at, area, None)
}

/// What the system sent the player of `client`: each message's title, whether the system sent it,
/// and what it holds.
fn sent(host: &mut Host, client: u32) -> Vec<(String, bool, Vec<ItemStack>)> {
    host.inbox(client)
        .rows
        .into_iter()
        .map(|row| (row.title, row.from == InboxSender::System, row.attached))
        .collect()
}

fn goals(plate: &AreaInstanceShown) -> Vec<(String, u32, u32)> {
    plate
        .goals
        .iter()
        .map(|goal| (goal.label.clone(), goal.done, goal.of))
        .collect()
}

fn roster(plate: &AreaInstanceShown) -> Vec<(String, RosterPlace)> {
    plate
        .roster
        .iter()
        .map(|row| (row.character.name.clone(), row.place))
        .collect()
}

fn rows(call: &InstanceCallShown) -> Vec<(&str, RosterRowShown)> {
    call.rows
        .iter()
        .map(|row| (row.character.name.as_str(), row.shown))
        .collect()
}

fn states(call: &InstanceCallShown) -> Vec<RosterRowShown> {
    call.rows.iter().map(|row| row.shown).collect()
}

fn too_far(host: &Host) -> RosterRowShown {
    RosterRowShown::TooFar {
        area: host.server.village.id::<AreaDef>("forest"),
    }
}

/// A game of a village and a forest everyone shares and three instanced areas: a vault for one
/// to `most` players, cleared by two wardens falling and its gate being sealed, with a door in
/// the village; an archive for two to three; and a trial for one that lasts ten minutes. Things
/// a player uses lead in and out and do what the scripts of instances do.
fn game(most: u32) -> Game {
    let stacked = |max: u32| json!({ "kind": { "type": "resource" }, "modules": [{ "type": "stack", "max": max }] });
    let place =
        |area: &str, at: Pos<Meters>| json!({ "area": path("area", area), "x": at.x, "y": at.y });
    let used = |game: Game, item: &str, script: &str, nodes: Value| {
        game.item(
            item,
            json!({
                "kind": { "type": "usable" },
                "modules": [{ "type": "script", "script": path("script", script) }],
            }),
        )
        .script(script, "item", nodes)
    };
    let pebble = json!({ "type": "items", "give": [{ "item": path("item", "pebble") }] });
    let spoils = json!({
        "xp": 50,
        "items": [
            { "item": path("item", "potion"), "amount": 2 },
            { "item": path("item", "gold"), "amount": 30 },
            { "item": path("item", "warden_key") },
        ],
    });
    let entry = |planks: u32| {
        json!({
            "needs": [{ "item": path("item", "plank"), "count": planks }],
            "empty": "Nothing is inserted.",
            "wrong": "{items} is not what it takes.",
            "right": "That is what it takes.",
        })
    };
    let instanced = |game: Game, area: &str, rules: Value| {
        game.area(area, &Map::open(12, 12), json!([]))
            .edit("area", area, |row| row["instance"] = rules)
    };
    let door = json!([{ "type": "resident", "npc": path("npc", "door"), "at": { "x": DOOR.x, "y": DOOR.y } }]);
    let game = Game::new()
        .area("village", &Map::open(12, 12), door)
        .area("forest", &Map::open(12, 12), json!([]))
        .hero("village", 4.0, 4.0)
        .sfx("ui_refuse")
        .sfx("ui_close")
        .interface_icon("map")
        .npc("warden", 25.0, json!([]))
        .npc(
            "door",
            25.0,
            json!([{ "type": "script", "script": path("script", "vault_gate") }]),
        )
        .vfx("damage_taken")
        .shader("damage_pop")
        .vfx("defeated")
        .item("potion", stacked(20))
        .item("gold", stacked(99))
        .item("plank", stacked(50))
        .item("warden_key", json!({ "kind": { "type": "resource" } }))
        .item("pebble", stacked(20))
        .edit_settings(|settings| {
            settings["lingers_for"] = json!(30.0);
            settings["slots"]["bag"] = json!(4);
            settings["party"] = json!({
                "size": 5,
                "invite_lasts": 60.0,
                "call_lasts": 120.0,
                "absent_for": 3600.0,
                "experience": { "split": "even", "range": 40.0, "bonus": 0.0 },
            });
            settings["mail"] = json!({
                "inbox": 2, "attachments": 6, "title_length": 60, "text_length": 1000, "a_minute": 2
            });
        });
    let game = instanced(
        game,
        "vault",
        json!({
            "players": { "least": 1, "most": most },
            "entry": entry(2),
            "cooldown": 60.0,
            "goals": [
                { "type": "defeat", "npc": path("npc", "warden"), "count": 2 },
                { "type": "task", "task": "seal", "label": "Seal the gate" },
            ],
            "empties_after": 300.0,
            "closes_after": 30.0,
        }),
    );
    let game = instanced(
        game,
        "archive",
        json!({
            "players": { "least": 2, "most": 3 },
            "entry": entry(3),
            "cooldown": 60.0,
            "goals": [{ "type": "task", "task": "read", "label": "Read the ledger" }],
            "empties_after": 300.0,
            "closes_after": 30.0,
        }),
    );
    let game = instanced(
        game,
        "trial",
        json!({
            "players": { "least": 1, "most": 1 },
            "entry": entry(1),
            "cooldown": 120.0,
            "goals": [{ "type": "task", "task": "endure", "label": "Endure" }],
            "empties_after": 300.0,
            "closes_after": 30.0,
            "lasts": 600.0,
        }),
    );
    let enter = |area: &str, rewards: Value, back: Value| {
        json!({
            "type": "enter_instance",
            "to": place(area, ENTRANCE),
            "rewards": rewards,
            "return_to": back,
        })
    };
    let on_use = |next: &str| json!({ "type": "on", "event": "used", "next": next });
    let back = place("village", BY_THE_DOOR);
    let mut at_gate = enter("vault", spoils.clone(), back.clone());
    at_gate["refused"] = json!("sorry");
    let game = game.script(
        "vault_gate",
        "npc",
        json!({
            "interacted": { "type": "on", "event": "interacted", "verb": "use", "reach": 2.0, "next": "seam" },
            "seam": {
                "type": "dialogue",
                "lines": [{ "by": { "type": "narrator" }, "text": ["Cold air breathes through the seam."] }],
                "choices": [{ "label": ["Go down."], "to": "enter" }],
            },
            "enter": at_gate,
            "sorry": pebble,
        }),
    );
    let game = used(
        game,
        "vault_key",
        "vault_door",
        json!({
            "used": on_use("enter"),
            "enter": enter("vault", spoils, back.clone()),
        }),
    );
    let mut by_token = enter(
        "vault",
        json!({ "xp": 5 }),
        place("forest", Pos::new(6.0, 6.0)),
    );
    by_token["refused"] = json!("sorry");
    let game = used(
        game,
        "token",
        "vault_token",
        json!({ "used": on_use("enter"), "enter": by_token, "sorry": pebble }),
    );
    let mut archive = enter("archive", json!({ "xp": 5 }), back.clone());
    archive["refused"] = json!("sorry");
    let game = used(
        game,
        "archive_key",
        "archive_door",
        json!({ "used": on_use("enter"), "enter": archive, "sorry": pebble }),
    );
    let game = used(
        game,
        "trial_key",
        "trial_door",
        json!({
            "used": on_use("enter"),
            "enter": enter("trial", json!({ "xp": 50 }), back),
        }),
    );
    let game = used(
        game,
        "seal",
        "gate_sealed",
        json!({
            "used": on_use("seal"),
            "seal": { "type": "complete_goal", "task": "seal" },
        }),
    );
    let game = used(
        game,
        "rope",
        "way_out",
        json!({ "used": on_use("leave"), "leave": { "type": "leave_instance" } }),
    );
    used(
        game,
        "compass",
        "where_am_i",
        json!({
            "used": on_use("look"),
            "look": { "type": "if", "test": { "type": "in_instance" }, "yes": "inside" },
            "inside": pebble,
        }),
    )
}
