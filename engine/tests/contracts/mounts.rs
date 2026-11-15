use bevy_ecs::prelude::*;
use engine::core::content::Content;
use engine::core::ground::{GroundSize, WaterSurface};
use engine::core::math::Pos;
use engine::core::units::{MeterPos, Meters, Seconds};
use engine::systems::area::{self, AreaDef};
use engine::systems::combat::{self, AttackRequest, Attitude};
use engine::systems::item::{self, UseItemRequest};
use engine::systems::mount::seat::MountRequest;
use engine::systems::mount::{self, MountDef, MountKind, Riding, SeatIndex};
use engine::systems::movement::{self, MoveRequest, MoveSeq, position};
use engine::systems::npc::{self, NpcDef};
use engine::systems::player::{self, AreaDrawn};
use engine::systems::probe::{ProbeGame, ProbeGameQuestion, answer};
use engine::systems::resource::{self, GatherRequest, ResourceDef, StopGathering, Working};
use engine::systems::script::Place;
use engine::systems::skill::{self, CastAim, SkillDef, UseSkillRequest};
use engine::systems::social_request::RequestAnswer;
use engine::systems::stat::{self, StatKind};
use engine::systems::{actor, reach, request_sources, script, status};
use serde_json::{Value, json};
use testkit::{APEX, BODY, Game, HEIGHTS, Map, Sim, path, rig_clips};

use crate::server::Server;

/// Where the village's lake begins: dry ground west of it, a metre of water east, over a bed
/// nobody walks from a metre in.
const SHORE: f32 = 14.0;
const BEACH: Pos<Meters> = Pos::new(10.0, 6.0);
const ROCK: Pos<Meters> = Pos::new(3.0, 3.0);
const OFFSHORE: Pos<Meters> = Pos::new(15.5, 6.0);
const FAR_OUT: Pos<Meters> = Pos::new(21.0, 6.0);
/// How long a shoal takes to give once.
const ANGLING: f32 = 1.0;

#[test]
fn a_mount_goes_where_its_kind_travels_and_is_left_through_what_called_it() {
    let mut server = server();
    let wren = server.sign_in(1, "Wren", "village");
    stand(&mut server, wren, BEACH);
    for item in ["bridle", "feather", "oar"] {
        server.village.give(wren, item, 1);
    }
    let speed =
        |server: &Server| stat::effective(&server.village.world, wren, StatKind::MovementSpeed);
    let on_foot = speed(&server);

    let slot = server.village.slot_of(wren, "bridle");
    server.send(1, UseItemRequest { slot });
    run(&mut server, 0.5);
    assert_eq!(
        (seat(&server, wren), speed(&server)),
        (Some(("mare", 0)), on_foot + 4.0),
        "an item that casts a mount's skill calls it, and what the mount's script does holds on its driver"
    );
    walk(&mut server, 1, ROCK);
    assert!(
        at(&server, wren).distance(ROCK) > Meters(0.5),
        "a riding mount is stopped by what stops feet"
    );
    cast(&mut server, 1, "call_mare", CastAim::None);
    assert_eq!(
        (
            seat(&server, wren),
            speed(&server),
            server.village.count(wren, "bridle")
        ),
        (None, on_foot, 1),
        "the skill that called the mount steps off it, and the item is kept"
    );

    cast(&mut server, 1, "call_crow", CastAim::None);
    walk(&mut server, 1, ROCK);
    assert!(
        seat(&server, wren) == Some(("crow", 0)) && at(&server, wren).distance(ROCK) < Meters(0.1),
        "a flying mount goes over what blocks feet"
    );
    walk(&mut server, 1, FAR_OUT);
    assert!(
        at(&server, wren).distance(FAR_OUT) < Meters(0.1),
        "and over water"
    );
    server.send(1, MountRequest::StepOff { at: None });
    assert_eq!(
        (
            seat(&server, wren),
            server.village.errors(1).last().cloned()
        ),
        (
            Some(("crow", 0)),
            Some("No ground to step onto within reach".to_owned())
        ),
        "nobody steps off where no ground is within the mount's reach"
    );
    walk(&mut server, 1, BEACH);
    cast(&mut server, 1, "call_crow", CastAim::None);
    assert_eq!(
        seat(&server, wren),
        None,
        "landing is no cast: the cooldown the call began does not hold it up"
    );
    cast(&mut server, 1, "call_crow", CastAim::None);
    assert!(
        seat(&server, wren).is_none()
            && server
                .village
                .errors(1)
                .last()
                .is_some_and(|said| said.contains("cooling down")),
        "and it starts no cooldown of its own: the call's still runs"
    );

    for (spot, refusal) in [
        (Pos::new(8.0, 6.0), "No water deep enough within reach"),
        (FAR_OUT, "That spot is too far away"),
    ] {
        cast(&mut server, 1, "call_boat", CastAim::Point(spot));
        assert_eq!(
            (
                seat(&server, wren),
                server.village.errors(1).last().cloned()
            ),
            (None, Some(refusal.to_owned())),
            "a boat is called onto water within the skill's range, never walked toward"
        );
    }
    cast(&mut server, 1, "call_boat", CastAim::Point(OFFSHORE));
    assert_eq!(
        (seat(&server, wren), at(&server, wren)),
        (Some(("boat", 0)), OFFSHORE),
        "a good spot launches: the caster is on the water there, aboard"
    );
    walk(&mut server, 1, BEACH);
    assert!(
        at(&server, wren).x >= SHORE,
        "a boat goes only where the water floats it, so it is never beached"
    );
    stand(&mut server, wren, OFFSHORE);
    cast(
        &mut server,
        1,
        "call_boat",
        CastAim::Point(Pos::new(3.0, 8.0)),
    );
    assert_eq!(
        (
            seat(&server, wren),
            server.village.errors(1).last().cloned()
        ),
        (
            Some(("boat", 0)),
            Some("No ground to step onto within reach".to_owned())
        ),
        "afloat, the skill that called the boat picks the ground to step onto, within its reach"
    );
    let ashore = Pos::new(12.5, 6.0);
    cast(&mut server, 1, "call_boat", CastAim::Point(ashore));
    assert_eq!(
        (seat(&server, wren), at(&server, wren)),
        (None, ashore),
        "the driver lands on the spot they picked, and the boat is gone"
    );
}

#[test]
fn a_mount_is_refused_before_anything_is_spent() {
    let mut server = server();
    let wren = server.sign_in(1, "Wren", "village");
    let hale = server.sign_in(2, "Hale", "forest");
    stand(&mut server, wren, BEACH);
    server.village.give(wren, "bridle", 1);
    server.village.give(wren, "feather", 1);
    server.village.give(wren, "raft", 1);
    server.forest.give(hale, "feather", 1);

    cast(&mut server, 2, "call_crow", CastAim::None);
    assert_eq!(
        (seat(&server, hale), server.forest.errors(2)),
        (None, vec!["Mounts of this kind are barred here".to_owned()]),
        "an area bars the kinds of mount it names"
    );
    server.later(31.0);
    assert_eq!(
        server.forest.errors(2).len(),
        1,
        "and a call refused began no cooldown to run out"
    );

    cast(&mut server, 1, "call_mare", CastAim::None);
    cast(&mut server, 1, "call_crow", CastAim::None);
    assert_eq!(
        (seat(&server, wren), server.village.errors(1)),
        (Some(("mare", 0)), vec!["Step off first".to_owned()]),
        "nobody calls a mount from the back of another"
    );
    cast(&mut server, 1, "call_mare", CastAim::None);

    cast(
        &mut server,
        1,
        "call_raft",
        CastAim::Point(Pos::new(8.0, 6.0)),
    );
    assert_eq!(
        (seat(&server, wren), server.village.count(wren, "raft")),
        (None, 1),
        "a single-use mount refused is not used up"
    );
    cast(&mut server, 1, "call_raft", CastAim::Point(OFFSHORE));
    assert_eq!(
        (seat(&server, wren), server.village.count(wren, "raft")),
        (Some(("boat", 0)), 0),
        "and is spent as it launches"
    );
}

#[test]
fn a_seat_is_asked_of_the_driver_and_lets_its_rider_do_what_it_allows() {
    let mut server = server();
    let wren = server.sign_in(1, "Wren", "village");
    let hale = server.sign_in(2, "Hale", "village");
    let isolde = server.sign_in(3, "Isolde", "village");
    let corvin = server.sign_in(4, "Corvin", "village");
    for (who, spot) in [
        (hale, BEACH),
        (wren, Pos::new(4.0, 6.0)),
        (isolde, Pos::new(11.0, 4.0)),
        (corvin, Pos::new(11.0, 8.0)),
    ] {
        stand(&mut server, who, spot);
        server.village.give(who, "tonic", 1);
    }
    server.village.give(hale, "oar", 1);
    cast(&mut server, 2, "call_boat", CastAim::Point(OFFSHORE));

    server.send(1, MountRequest::AskSeat { driver: hale });
    assert_eq!(
        server.cards(2),
        [],
        "a seat is asked for from beside the mount, not from across the beach"
    );
    run(&mut server, 4.0);
    assert!(at(&server, wren).distance(OFFSHORE) <= Meters(4.0));
    assert_eq!(
        (server.cards(1), server.cards(2)),
        (
            vec![("You asked Hale for a seat".to_owned(), true)],
            vec![("Wren asks for a seat on your Rowboat".to_owned(), false)]
        ),
        "the asker walks into the mount's reach, and the driver is asked there"
    );
    assert_eq!(
        seat(&server, wren),
        None,
        "nobody boards without the driver's yes"
    );
    server.answer(2, RequestAnswer::Accept);
    server.step();
    assert_eq!(
        (seat(&server, wren), at(&server, wren)),
        (Some(("boat", 1)), OFFSHORE),
        "a yes seats the asker in the first free seat, where the driver is"
    );

    let onward = Pos::new(18.0, 6.0);
    walk(&mut server, 1, BEACH);
    assert_eq!(at(&server, wren), OFFSHORE, "a passenger steers nothing");
    walk(&mut server, 2, onward);
    assert_eq!(
        (at(&server, hale), at(&server, wren)),
        (onward, onward),
        "and goes wherever their driver does"
    );
    stand(&mut server, hale, OFFSHORE);
    server.step();

    for client in [1, 2] {
        let who = server.village.player(client);
        let slot = server.village.slot_of(who, "tonic");
        server.send(client, UseItemRequest { slot });
        run(&mut server, 0.5);
    }
    assert_eq!(
        (
            server.village.count(wren, "tonic"),
            server.village.count(hale, "tonic"),
            server.village.errors(2).last().cloned(),
            seat(&server, hale),
        ),
        (
            0,
            1,
            Some("Not from the Rowboat".to_owned()),
            Some(("boat", 0))
        ),
        "a seat lets its rider do what it lists, and a mount that refuses the rest leaves them seated"
    );

    for client in [3, 4] {
        server.send(client, MountRequest::AskSeat { driver: hale });
    }
    run(&mut server, 2.0);
    assert_eq!(server.cards(2).len(), 2, "two ask for the one seat left");
    server.answer(2, RequestAnswer::Accept);
    server.step();
    server.answer(2, RequestAnswer::Accept);
    server.step();
    assert_eq!(
        (
            seat(&server, isolde),
            seat(&server, corvin),
            server.village.errors(4)
        ),
        (
            Some(("boat", 2)),
            None,
            vec!["Every seat is taken".to_owned()]
        ),
        "a seat that went meanwhile is not given twice"
    );
    server.send(4, MountRequest::AskSeat { driver: hale });
    assert_eq!(
        server.village.errors(4),
        ["Every seat is taken"; 2],
        "and a full mount is not asked"
    );

    server.send(2, MountRequest::SetDown { seat: SeatIndex(2) });
    server.send(3, MountRequest::AskSeat { driver: hale });
    run(&mut server, 1.0);
    assert_eq!(server.cards(2).len(), 1);
    walk(&mut server, 3, Pos::new(4.0, 6.0));
    assert_eq!(
        (server.cards(2), seat(&server, isolde)),
        (Vec::new(), None),
        "walking out of the mount's reach takes the request back"
    );

    let bryn = server.sign_in(5, "Bryn", "village");
    stand(&mut server, bryn, Pos::new(6.0, 9.0));
    server.village.give(bryn, "bridle", 1);
    server.village.equip(bryn, "sword");
    cast(&mut server, 5, "call_mare", CastAim::None);
    let village = server.village.id::<AreaDef>("village");
    let dummy = server.village.id::<NpcDef>("dummy");
    let dummy = npc::spawn(
        &mut server.village.world,
        dummy,
        Pos::new(8.0, 9.0),
        village,
        None,
    );
    server
        .village
        .world
        .entity_mut(dummy)
        .insert(Attitude::Hostile);
    server.step();
    assert_eq!(seat(&server, bryn), Some(("mare", 0)));
    server.send(5, AttackRequest { target: dummy });
    assert_eq!(
        (
            seat(&server, bryn),
            reach::attacking(&server.village.world, bryn)
        ),
        (None, Some(dummy)),
        "a mount that has its rider step off for what the seat bars sets them down and carries on"
    );
}

#[test]
fn everyone_aboard_works_what_is_in_reach_of_their_mount_until_it_moves_on() {
    let mut server = server();
    let wren = server.sign_in(1, "Wren", "village");
    let hale = server.sign_in(2, "Hale", "village");
    let isolde = server.sign_in(3, "Isolde", "village");
    aboard(&mut server, (2, hale), (1, wren));
    stand(&mut server, isolde, Pos::new(11.0, 5.0));
    board(&mut server, (2, hale), (3, isolde));
    for angler in [wren, hale, isolde] {
        server.village.give(angler, "lure", 2);
    }
    let village = server.village.id::<AreaDef>("village");
    let shoal = server.village.id::<ResourceDef>("shoal");
    let [near, far] = [18.5, 23.0]
        .map(|x| resource::spawn(&mut server.village.world, shoal, Pos::new(x, 6.0), village));
    let working = |server: &Server, who: Entity| {
        server
            .village
            .world
            .get::<Working>(who)
            .map(|working| working.resource)
    };
    let lures = |server: &Server| [wren, hale].map(|angler| server.village.count(angler, "lure"));

    for client in [1, 2, 3] {
        server.send(client, GatherRequest { target: near });
    }
    run(&mut server, 0.3);
    assert_eq!(
        (
            working(&server, hale),
            working(&server, wren),
            working(&server, isolde),
            server.village.errors(3).last().cloned(),
            at(&server, hale),
        ),
        (
            Some(near),
            Some(near),
            None,
            Some("Not from the Rowboat".to_owned()),
            OFFSHORE
        ),
        "the driver and every passenger whose seat lets them work what the mount is in reach of, from where it lies"
    );
    walk(&mut server, 2, Pos::new(16.5, 6.0));
    assert_eq!(
        (
            working(&server, hale),
            working(&server, wren),
            lures(&server)
        ),
        (None, None, [2, 2]),
        "the mount moving on ends the work of everyone aboard, and nothing is spent"
    );

    server.send(1, GatherRequest { target: far });
    run(&mut server, 1.0);
    assert_eq!(
        (working(&server, wren), at(&server, wren)),
        (None, Pos::new(16.5, 6.0)),
        "a passenger steers nothing toward what is out of reach"
    );
    server.send(2, GatherRequest { target: far });
    for _ in 0..40 {
        if [wren, hale].map(|angler| working(&server, angler)) == [Some(far); 2] {
            break;
        }
        run(&mut server, 0.2);
    }
    assert!(at(&server, hale).distance(Pos::new(23.0, 6.0)) <= Meters(4.0));
    run(&mut server, ANGLING + 0.3);
    assert_eq!(
        (
            working(&server, hale),
            working(&server, wren),
            lures(&server),
            [wren, hale].map(|angler| server.village.count(angler, "trout")),
        ),
        (Some(far), Some(far), [1; 2], [1; 2]),
        "the driver takes the mount into reach, where the passenger who asked works it too, each from their own bag"
    );
}

#[test]
fn every_way_off_a_mount_sets_its_rider_on_ground_they_can_stand_on() {
    enum Leaves {
        ByChoice,
        SetDown,
        Blocking,
        DriverFalls,
        DriverGone,
        PlayerGone,
    }
    for way in [
        Leaves::ByChoice,
        Leaves::SetDown,
        Leaves::Blocking,
        Leaves::DriverFalls,
        Leaves::DriverGone,
        Leaves::PlayerGone,
    ] {
        let mut server = server();
        let wren = server.sign_in(1, "Wren", "village");
        let hale = server.sign_in(2, "Hale", "village");
        aboard(&mut server, (2, hale), (1, wren));
        walk(&mut server, 2, FAR_OUT);
        assert_eq!(at(&server, wren), FAR_OUT);
        let (seated, driving, said) = match way {
            Leaves::ByChoice => {
                server.send(1, MountRequest::StepOff { at: None });
                (true, true, "No ground to step onto within reach")
            }
            Leaves::SetDown => {
                server.send(2, MountRequest::SetDown { seat: SeatIndex(1) });
                (false, true, "Hale set you down")
            }
            Leaves::Blocking => {
                server.block(1, "Hale", true);
                server.step();
                (false, true, "")
            }
            Leaves::DriverFalls => {
                combat::inflict(&mut server.village.world, hale, None, 1_000_000.0);
                run(&mut server, 0.5);
                (false, false, "")
            }
            Leaves::DriverGone => {
                server.village.disconnect(2);
                run(&mut server, 0.5);
                (false, false, "")
            }
            Leaves::PlayerGone => {
                server.village.disconnect(1);
                run(&mut server, 0.5);
                (false, true, "")
            }
        };
        assert_eq!(
            (seat(&server, wren).is_some(), seat(&server, hale).is_some()),
            (seated, driving),
            "who still rides"
        );
        assert_eq!(
            walkable(&server, at(&server, wren)),
            !seated,
            "whoever is set down stands on ground a walker stands on, however far that is"
        );
        if !said.is_empty() {
            assert_eq!(
                server.told_or_refused(1).last().map(String::as_str),
                Some(said)
            );
        }
    }
}

#[test]
fn a_warp_takes_a_mount_and_everyone_on_it_or_leaves_the_mount_behind() {
    let landing = Pos::new(4.0, 4.0);
    let pace = |sim: &Sim, who: Entity| stat::effective(&sim.world, who, StatKind::MovementSpeed);
    let mut server = server();
    let wren = server.sign_in(1, "Wren", "village");
    let hale = server.sign_in(2, "Hale", "village");
    let on_foot = pace(&server.village, hale);
    server.village.give(hale, "bridle", 1);
    stand(&mut server, hale, BEACH);
    stand(&mut server, wren, Pos::new(9.0, 6.0));
    cast(&mut server, 2, "call_mare", CastAim::None);
    board(&mut server, (2, hale), (1, wren));
    warp(&mut server.village, hale, "forest", landing);
    server.village.hand_over(&mut server.forest);
    server.step();
    let (hale, wren) = (server.forest.player(2), server.forest.player(1));
    assert_eq!(
        (
            rides(&server.forest, hale),
            rides(&server.forest, wren),
            server
                .forest
                .world
                .get::<Riding>(wren)
                .map(|riding| riding.driver),
            position(&server.forest.world, wren),
            pace(&server.forest, hale),
        ),
        (
            Some(("mare", 0)),
            Some(("mare", 1)),
            Some(hale),
            position(&server.forest.world, hale),
            on_foot + 4.0,
        ),
        "a warp takes the driver and every passenger through, each to the seat they held, and what the mount's script does holds on"
    );

    warp(&mut server.forest, wren, "forest", Pos::new(8.0, 8.0));
    server.step();
    assert_eq!(
        (rides(&server.forest, hale), rides(&server.forest, wren)),
        (Some(("mare", 0)), None),
        "a passenger warped alone leaves their seat, and the mount stays with its driver"
    );

    warp(&mut server.forest, hale, "village", FAR_OUT);
    server.forest.hand_over(&mut server.village);
    server.step();
    let hale = server.village.player(2);
    assert_eq!(
        (
            seat(&server, hale),
            walkable(&server, at(&server, hale)),
            pace(&server.village, hale),
            server.told(2).last().map(String::as_str),
        ),
        (None, true, on_foot, Some("Your Grey Mare could not follow")),
        "a mount that cannot stand where its driver lands in another area is left behind, with all it did"
    );

    let mut server = server_of(game());
    let wren = server.sign_in(1, "Wren", "village");
    let hale = server.sign_in(2, "Hale", "village");
    aboard(&mut server, (2, hale), (1, wren));
    warp(&mut server.village, hale, "village", landing);
    for client in [1, 2] {
        server.send(client, AreaDrawn);
    }
    for (client, rider) in [(1, wren), (2, hale)] {
        assert!(
            seat(&server, rider).is_none()
                && walkable(&server, at(&server, rider))
                && at(&server, rider).distance(landing) < Meters(2.0),
            "where the mount cannot stand, everyone arrives on foot around the landing"
        );
        assert_eq!(
            server.told(client).last().map(String::as_str),
            Some("Your Rowboat could not follow")
        );
    }

    server.village.give(hale, "feather", 1);
    cast(&mut server, 2, "call_crow", CastAim::None);
    warp(&mut server.village, hale, "forest", landing);
    server.village.hand_over(&mut server.forest);
    server.step();
    let hale = server.forest.player(2);
    assert_eq!(
        (rides(&server.forest, hale), server.told(2).last().cloned()),
        (None, Some("Your Stormcrow could not follow".to_owned())),
        "and so does a rider who warps into an area that bars their mount"
    );
}

#[test]
fn the_game_answers_where_each_of_its_mounts_goes() {
    let content = game().engine_rows().load();
    let listed: ProbeGame = asked(&content, &ProbeGameQuestion::Game);
    let mut mounts: Vec<(String, MountKind, usize, Vec<String>)> = listed
        .mounts
        .iter()
        .map(|mount| {
            let callers = mount
                .called_by
                .iter()
                .map(|skill| skill.get(&content).name.clone());
            (
                mount.name.clone(),
                mount.kind,
                mount.seats,
                callers.collect(),
            )
        })
        .collect();
    mounts.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        mounts,
        [
            (
                "Grey Mare".to_owned(),
                MountKind::Riding,
                2,
                vec!["call_mare".to_owned()]
            ),
            (
                "Rowboat".to_owned(),
                MountKind::Floating,
                3,
                vec!["call_boat".to_owned(), "call_raft".to_owned()]
            ),
            (
                "Stormcrow".to_owned(),
                MountKind::Flying,
                1,
                vec!["call_crow".to_owned()]
            ),
        ]
    );
    let village = content
        .by_path::<AreaDef>(&path("area", "village"))
        .expect("the village");
    let forest = content
        .by_path::<AreaDef>(&path("area", "forest"))
        .expect("the forest");
    let goes = |mount: Option<&str>, area, at| -> bool {
        let riding = mount.and_then(|mount| content.by_path::<MountDef>(&path("mount", mount)));
        asked(&content, &ProbeGameQuestion::Walkable { area, riding, at })
    };
    let everywhere = [BEACH, ROCK, FAR_OUT];
    for (mount, area, wanted) in [
        (None, village, [true, false, false]),
        (Some("mare"), village, [true, false, false]),
        (Some("crow"), village, [true, true, true]),
        (Some("boat"), village, [false, false, true]),
        (Some("crow"), forest, [false, false, false]),
    ] {
        assert_eq!(
            everywhere.map(|at| goes(mount, area, at)),
            wanted,
            "where {mount:?} goes on the beach, the rock and the lake"
        );
    }
    assert!(
        !goes(Some("boat"), village, Pos::new(SHORE + 0.5, 1.0)),
        "a boat floats only where the water is as deep as it draws"
    );
}

fn asked<Answer: serde::de::DeserializeOwned>(
    content: &Content,
    question: &ProbeGameQuestion,
) -> Answer {
    use engine::core::content::{ContentScope, with_content};
    let line = answer(content, question);
    with_content(ContentScope::of(content), || serde_json::from_str(&line))
        .0
        .unwrap_or_else(|error| panic!("an unreadable answer {line}: {error}"))
}

/// A village with a rock to walk around and a lake east of its beach, shallow along the north
/// bank and closed to feet past its shallows, and a forest that bars whatever flies; a mare, a
/// crow and a boat to ride, each called by a skill an item casts.
fn game() -> Game {
    let mut clips = rig_clips();
    clips.extend(["sit", "row", "angle"].map(|made| (made, Seconds(0.4))));
    let lands = |clip: &str| json!({ "clip": testkit::clip(clip), "cues": [{ "at": APEX.0, "apex": true }] });
    let calls = |name: &str, mount: &str, fields: Value| {
        let mut row = json!({
            "name": name,
            "icon": "icons/skill.png",
            "levels": [{ "cost": 1, "stats": {} }],
            "modules": [{ "type": "script", "script": path("script", &format!("{name}_used")) }],
        });
        for (field, value) in fields.as_object().expect("a skill's fields") {
            row[field] = value.clone();
        }
        let script = json!({
            "used": { "type": "on", "event": "used", "next": "call" },
            "call": { "type": "mount", "mount": path("mount", mount) },
        });
        (row, script)
    };
    let casts =
        |skill: &str| json!({ "kind": { "type": "usable", "casts": path("skill", skill) } });
    let seat = |joint: &str, rides: &str, may: Value| json!({ "joint": joint, "rides": rides, "may": may });
    let water = WaterSurface::from_fn(
        GroundSize {
            width: 24,
            height: 12,
        },
        HEIGHTS,
        |column, row| {
            let deep = if row < 4 { 0.2 } else { 1.0 };
            Meters(if column as f32 >= SHORE * 2.0 {
                deep
            } else {
                0.0
            })
        },
    );
    let mut rows = vec![format!("{}{}", ".".repeat(15), "#".repeat(9)); 12];
    for row in &mut rows[2..4] {
        row.replace_range(2..4, "##");
    }
    let rows: Vec<&str> = rows.iter().map(String::as_str).collect();
    let game = Game::new()
        .skeleton_of(
            BODY,
            &clips,
            json!({ "cues": [lands("attack"), lands("angle")] }),
        )
        .edit("skeleton", BODY, |body| {
            body["base"]["moves"] = json!({
                "sit": testkit::clip("sit"),
                "row": testkit::clip("row"),
                "angle": testkit::clip("angle"),
            });
        })
        .area("village", &Map::of(&rows), json!([]))
        .area("forest", &Map::open(12, 12), json!([]))
        .edit("area", "forest", |forest| {
            forest["bars_mounts"] = json!(["flying"])
        })
        .hero("village", 9.0, 6.0)
        .sword()
        .sfx("ui_refuse")
        .vfx("damage_taken")
        .shader("damage_pop")
        .vfx("defeated")
        .npc("dummy", 100.0, json!([]))
        .asset("icons/skill.png", testkit::png(1, 1))
        .edit_settings(|settings| settings["lingers_for"] = json!(30.0))
        .model("mare", 2.0, json!([]))
        .model("crow", 2.0, json!([]))
        .model("boat", 1.0, json!([]))
        .row(
            "mount",
            "mare",
            json!({
                "name": "Grey Mare",
                "look": path("model", "mare"),
                "travels": { "type": "riding" },
                "reach": 3.0,
                "seats": [seat("body", "sit", json!(["items"])), seat("hand.r", "sit", json!([]))],
                "barred": "step_off",
                "modules": [{ "type": "script", "script": path("script", "mare_ridden") }],
            }),
        )
        .script(
            "mare_ridden",
            "mount",
            json!({
                "activated": { "type": "on", "event": "activated", "next": "quicken" },
                "quicken": { "type": "boost", "stat": "movement_speed", "by": 4.0 },
            }),
        )
        .row(
            "mount",
            "crow",
            json!({
                "name": "Stormcrow",
                "look": path("model", "crow"),
                "travels": { "type": "flying", "height": 6.0, "eases": 1.0 },
                "reach": 3.0,
                "seats": [seat("body", "sit", json!([]))],
                "barred": "refuse",
            }),
        )
        .row(
            "mount",
            "boat",
            json!({
                "name": "Rowboat",
                "look": path("model", "boat"),
                "travels": { "type": "floating", "draught": 0.5 },
                "reach": 4.0,
                "seats": [
                    seat("body", "row", json!(["gather"])),
                    seat("hand.r", "sit", json!(["items", "skills", "gather"])),
                    seat("hand.l", "sit", json!(["items", "skills"])),
                ],
                "barred": "refuse",
            }),
        )
        .row(
            "tool_type",
            "line",
            json!({
                "name": "Line",
                "by_hand": true,
                "work": "angle",
                "cursor": path("cursor", "hook"),
                "sfx": { "swing": path("sfx", "plop"), "ran_out": path("sfx", "plop") },
            }),
        )
        .cursor("hook")
        .sfx("plop")
        .prop("ripples", json!({ "type": "none" }))
        .resource("lure", 20)
        .resource("trout", 20)
        .row(
            "resource",
            "shoal",
            json!({
                "name": "Shoal",
                "look": path("prop", "ripples"),
                "quantity": 9,
                "max_occupants": 3,
                "tool": path("tool_type", "line"),
                "spends": [{ "item": path("item", "lure") }],
                "reach": 4.0,
                "takes": ANGLING,
                "drops": [{ "item": path("item", "trout") }],
            }),
        )
        .item("bridle", casts("call_mare"))
        .item("feather", casts("call_crow"))
        .item("oar", casts("call_boat"))
        .item("raft", casts("call_raft"))
        .item(
            "tonic",
            json!({
                "kind": { "type": "usable" },
                "modules": [{ "type": "script", "script": path("script", "tonic_drunk") }],
            }),
        )
        .script(
            "tonic_drunk",
            "item",
            json!({
                "used": { "type": "on", "event": "used", "next": "drink" },
                "drink": { "type": "items", "take": [{ "item": path("item", "tonic") }] },
            }),
        );
    let game = [
        calls("call_mare", "mare", json!({ "target": "self" })),
        calls(
            "call_crow",
            "crow",
            json!({ "target": "self", "cooldown": 30.0 }),
        ),
        calls(
            "call_boat",
            "boat",
            json!({ "target": "spot", "range": 6.0 }),
        ),
        calls(
            "call_raft",
            "boat",
            json!({ "target": "spot", "range": 6.0, "cost": [{ "item": path("item", "raft") }] }),
        ),
    ]
    .into_iter()
    .fold(game, |game, (row, script)| {
        let name = row["name"].as_str().expect("a skill's name").to_owned();
        game.row("skill", &name, row)
            .script(&format!("{name}_used"), "skill", script)
    });
    game.water("village", &water)
}

fn server() -> Server {
    server_of(game())
}

/// A server of `game` whose two worlds play what mounts take: walks, casts, items, seats, and
/// the scripts and statuses a mount's own script runs through.
fn server_of(game: Game) -> Server {
    let mut server = Server::of(game, request_sources());
    for sim in [&mut server.village, &mut server.forest] {
        sim.accept::<MoveRequest>()
            .accept::<AreaDrawn>()
            .accept::<AttackRequest>()
            .accept::<UseSkillRequest>()
            .accept::<UseItemRequest>()
            .accept::<MountRequest>()
            .accept::<GatherRequest>()
            .accept::<StopGathering>()
            .add((
                player::arrival_holds,
                movement::move_request,
                resource::gather_request,
                combat::request,
                skill::casting::request,
                item::use_item,
                mount::seat::requests,
            ))
            .plan((
                status::aspect::SettleAspects::default(),
                status::aspect::TickAspects::default(),
            ))
            .plan(skill::casting::StepCasts::default())
            .plan((
                script::query::ListRoster::default(),
                script::RunFlows::default(),
                movement::AdvanceWalks::default(),
            ))
            .plan(mount::CarryRiders::default())
            .plan(stat::SettleVitals::default())
            .add((
                mount::seat::boarding,
                resource::gathering,
                resource::work,
                combat::defeats,
                skill::hotbar::settle_basic_attack,
            ))
            .plan(actor::SettleActions::default());
        script::spawn_stateless(&mut sim.world);
    }
    server
}

impl Server {
    /// The last things the player was told, refusals among them.
    fn told_or_refused(&mut self, client: u32) -> Vec<String> {
        let sim = self.village_or_forest(client);
        sim.told(client)
            .into_iter()
            .map(|told| told.text.plain_words())
            .collect()
    }
}

/// `driver` launches the boat just off the beach, and `passenger` is given a seat on it.
fn aboard(server: &mut Server, driver: (u32, Entity), passenger: (u32, Entity)) {
    stand(server, driver.1, BEACH);
    stand(server, passenger.1, Pos::new(11.0, 6.0));
    server.village.give(driver.1, "oar", 1);
    cast(server, driver.0, "call_boat", CastAim::Point(OFFSHORE));
    board(server, driver, passenger);
}

/// `passenger` asks `driver` for a seat from within the mount's reach, and is given it.
fn board(server: &mut Server, driver: (u32, Entity), passenger: (u32, Entity)) {
    server.send(passenger.0, MountRequest::AskSeat { driver: driver.1 });
    run(server, 2.0);
    server.answer(driver.0, RequestAnswer::Accept);
    server.step();
    assert!(seat(server, passenger.1).is_some(), "a passenger aboard");
}

fn run(server: &mut Server, seconds: f32) {
    let ticks = (seconds * engine::systems::TICK_HZ.0).ceil() as usize;
    for _ in 0..ticks {
        server.step();
    }
}

fn cast(server: &mut Server, client: u32, skill: &str, aim: CastAim) {
    let skill = server.village.id::<SkillDef>(skill);
    server.send(client, UseSkillRequest { skill, aim });
    run(server, 0.5);
}

/// The player asks to go to `to`, and the world plays long enough for anyone to get there.
fn walk(server: &mut Server, client: u32, to: Pos<Meters>) {
    static ASKED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    let seq = MoveSeq(ASKED.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    server.send(client, MoveRequest { pos: to, seq });
    run(server, 6.0);
}

/// Stages `who` at `at`, as a test sets its scene.
fn stand(server: &mut Server, who: Entity, at: Pos<Meters>) {
    movement::relocate(&mut server.village.world, who, at);
    server.step();
}

fn warp(sim: &mut Sim, who: Entity, area: &str, at: Pos<Meters>) {
    let to = Place {
        area: sim.id::<AreaDef>(area),
        at,
    };
    area::warp::warp(&mut sim.world, who, to).expect("a warp");
}

fn at(server: &Server, who: Entity) -> Pos<Meters> {
    position(&server.village.world, who).expect("a character in the village")
}

/// The mount `who` rides in the village and the seat they hold on it.
fn seat(server: &Server, who: Entity) -> Option<(&'static str, u8)> {
    rides(&server.village, who)
}

fn rides(sim: &Sim, who: Entity) -> Option<(&'static str, u8)> {
    let riding = sim.world.get::<Riding>(who)?;
    let mount = ["mare", "crow", "boat"]
        .into_iter()
        .find(|mount| sim.id::<MountDef>(mount) == riding.mount)?;
    Some((mount, riding.seat.0))
}

fn walkable(server: &Server, at: Pos<Meters>) -> bool {
    let content = server.village.world.resource::<Content>();
    let village = server.village.id::<AreaDef>("village");
    area::load(content, village).mesh().walkable(at)
}
