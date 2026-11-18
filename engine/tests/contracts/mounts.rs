use bevy_ecs::prelude::*;
use engine::core::content::Content;
use engine::core::ground::{GroundSize, WaterSurface};
use engine::core::math::Pos;
use engine::core::units::{MeterPos, Meters, Seconds};
use engine::systems::area::Place;
use engine::systems::area::{self, AreaDef};
use engine::systems::combat::{self, AttackRequest, Attitude};
use engine::systems::item::{self, UseItemRequest};
use engine::systems::mount::seat::MountRequest;
use engine::systems::mount::{self, MountDef, MountKind, Riding};
use engine::systems::movement::{self, MoveRequest, MoveSeq, Position, position};
use engine::systems::npc::{self, NpcDef};
use engine::systems::player::snapshot::{CharacterSnapshot, JoinAs, JoinOrigin, SnapshotSpot};
use engine::systems::player::{self, AreaDrawn};
use engine::systems::probe::{ProbeGame, ProbeGameQuestion, answer};
use engine::systems::resource::{self, GatherRequest, ResourceDef, StopGathering, Working};
use engine::systems::skill::{self, CastAim, SkillDef, UseSkillRequest};
use engine::systems::stat::{self, StatKind};
use engine::systems::{actor, reach, request_sources, status};
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
        (Some("mare"), on_foot + 4.0),
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
        seat(&server, wren) == Some("crow") && at(&server, wren).distance(ROCK) < Meters(0.1),
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
            Some("crow"),
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
    let jumps = |server: &Server| {
        let placed = server.village.world.get::<Position>(wren);
        placed.expect("placed").jumps
    };
    let ashore_jumps = jumps(&server);
    cast(&mut server, 1, "call_boat", CastAim::Point(OFFSHORE));
    assert_eq!(
        (seat(&server, wren), at(&server, wren), jumps(&server)),
        (Some("boat"), OFFSHORE, ashore_jumps),
        "a good spot launches: the caster is on the water there, aboard, with no jump to cut their view for"
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
            Some("boat"),
            Some("No ground to step onto within reach".to_owned())
        ),
        "afloat, the skill that called the boat picks the ground to step onto, within its reach"
    );
    let ashore = Pos::new(12.5, 6.0);
    let afloat_jumps = jumps(&server);
    cast(&mut server, 1, "call_boat", CastAim::Point(ashore));
    assert_eq!(
        (seat(&server, wren), at(&server, wren), jumps(&server)),
        (None, ashore, afloat_jumps),
        "the driver lands on the spot they picked, the boat is gone, and stepping off is no jump either"
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
        (None, vec!["This area does not allow mounts".to_owned()]),
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
        (Some("mare"), vec!["Step off first".to_owned()]),
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
        (Some("boat"), 0),
        "and is spent as it launches"
    );
}

#[test]
fn a_seat_lets_its_rider_do_what_it_allows() {
    let mut server = server();
    let hale = server.sign_in(2, "Hale", "village");
    let wren = server.sign_in(1, "Wren", "village");
    for who in [hale, wren] {
        server.village.give(who, "tonic", 1);
    }
    stand(&mut server, hale, BEACH);
    server.village.give(hale, "feather", 1);
    cast(&mut server, 2, "call_crow", CastAim::None);
    stand(&mut server, wren, Pos::new(8.0, 9.0));
    server.village.give(wren, "bridle", 1);
    cast(&mut server, 1, "call_mare", CastAim::None);

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
            seat(&server, wren),
            seat(&server, hale),
        ),
        (
            0,
            1,
            Some("Not from the Stormcrow".to_owned()),
            Some("mare"),
            Some("crow")
        ),
        "a seat lets its rider do what it lists, and a mount that refuses the rest leaves them seated"
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
        Pos::new(8.0, 11.0),
        village,
        None,
    );
    server
        .village
        .world
        .entity_mut(dummy)
        .insert(Attitude::Hostile);
    server.step();
    assert_eq!(seat(&server, bryn), Some("mare"));
    server.send(5, AttackRequest { target: dummy });
    assert_eq!(
        (
            seat(&server, bryn),
            reach::attacking(&server.village.world, bryn)
        ),
        (None, Some(dummy)),
        "a mount that has its rider step off for what the seat bars lets them off and carries on"
    );
}

#[test]
fn a_rider_works_what_is_in_reach_of_their_mount_until_it_moves_on() {
    let mut server = server();
    let hale = server.sign_in(2, "Hale", "village");
    launch(&mut server, 2, hale);
    server.village.give(hale, "lure", 2);
    let village = server.village.id::<AreaDef>("village");
    let shoal = server.village.id::<ResourceDef>("shoal");
    let [near, far] = [18.5, 23.0]
        .map(|x| resource::spawn(&mut server.village.world, shoal, Pos::new(x, 6.0), village));
    let working = |server: &Server| {
        server
            .village
            .world
            .get::<Working>(hale)
            .map(|working| working.resource)
    };

    server.send(2, GatherRequest { target: near });
    run(&mut server, 0.3);
    assert_eq!(
        (working(&server), at(&server, hale)),
        (Some(near), OFFSHORE),
        "a rider whose seat lets them works what the mount is in reach of, from where it lies"
    );
    walk(&mut server, 2, Pos::new(16.5, 6.0));
    assert_eq!(
        (working(&server), server.village.count(hale, "lure")),
        (None, 2),
        "the mount moving on ends the work, and nothing is spent"
    );

    server.send(2, GatherRequest { target: far });
    for _ in 0..40 {
        if working(&server) == Some(far) {
            break;
        }
        run(&mut server, 0.2);
    }
    assert!(at(&server, hale).distance(Pos::new(23.0, 6.0)) <= Meters(4.0));
    run(&mut server, ANGLING + 0.3);
    assert_eq!(
        (
            working(&server),
            server.village.count(hale, "lure"),
            server.village.count(hale, "trout")
        ),
        (Some(far), 1, 1),
        "the rider takes the mount into reach of what is farther off, and works it there"
    );
}

#[test]
fn a_rider_leaves_their_mount_only_onto_ground_within_its_reach() {
    enum Way {
        StepsOffAfloat,
        StepsOffAshore,
        Dismounted,
        Falls,
        PlayerGone,
    }
    for way in [
        Way::StepsOffAfloat,
        Way::StepsOffAshore,
        Way::Dismounted,
        Way::Falls,
        Way::PlayerGone,
    ] {
        let mut server = server();
        let hale = server.sign_in(2, "Hale", "village");
        launch(&mut server, 2, hale);
        let out = match way {
            Way::StepsOffAshore => OFFSHORE,
            _ => FAR_OUT,
        };
        walk(&mut server, 2, out);
        let (rides, lands, said, case) = match way {
            Way::StepsOffAfloat => {
                server.send(2, MountRequest::StepOff { at: None });
                (
                    true,
                    false,
                    "No ground to step onto within reach",
                    "stepping off with no ground within the mount's reach is refused",
                )
            }
            Way::StepsOffAshore => {
                server.send(2, MountRequest::StepOff { at: None });
                (
                    false,
                    true,
                    "",
                    "with ground within reach, the rider steps onto it",
                )
            }
            Way::Dismounted => {
                server.village.give(hale, "shell", 1);
                let slot = server.village.slot_of(hale, "shell");
                server.send(2, UseItemRequest { slot });
                run(&mut server, 0.5);
                (
                    true,
                    false,
                    "No ground to step onto within reach",
                    "a script takes a rider off as stepping off does, and tells them why it can't",
                )
            }
            Way::Falls => {
                combat::inflict(&mut server.village.world, hale, None, 1_000_000.0);
                run(&mut server, 0.5);
                (
                    false,
                    false,
                    "",
                    "a fallen rider leaves the mount where they fell, moved nowhere",
                )
            }
            Way::PlayerGone => {
                server.village.disconnect(2);
                run(&mut server, 0.5);
                (
                    true,
                    false,
                    "",
                    "a rider whose player is gone stays where they are, on the mount",
                )
            }
        };
        let there = at(&server, hale);
        assert_eq!(
            (seat(&server, hale).is_some(), walkable(&server, there)),
            (rides, lands),
            "{case}"
        );
        if !lands {
            assert_eq!(there, out, "{case}: nobody is moved");
        }
        if !said.is_empty() {
            assert_eq!(
                server.told_or_refused(2).last().map(String::as_str),
                Some(said),
                "{case}"
            );
        }
    }
}

#[test]
fn a_warp_keeps_its_rider_mounted_only_where_the_mount_may_be_at_the_spot_it_leads_to() {
    let landing = Pos::new(4.0, 4.0);
    let pace = |sim: &Sim, who: Entity| stat::effective(&sim.world, who, StatKind::MovementSpeed);
    let told = |sim: &mut Sim, client: u32| -> Option<String> {
        sim.told(client)
            .last()
            .map(|told| told.text.plain_words(sim.content()))
    };
    let not_here = "The Grey Mare can't go where you arrived. You're on foot.";

    let mut server = server();
    let hale = server.sign_in(2, "Hale", "village");
    let on_foot = pace(&server.village, hale);
    server.village.give(hale, "bridle", 1);
    stand(&mut server, hale, BEACH);
    cast(&mut server, 2, "call_mare", CastAim::None);
    warp(&mut server.village, hale, "forest", landing);
    server.village.hand_over(&mut server.forest);
    server.step();
    let hale = server.forest.player(2);
    assert_eq!(
        (rides(&server.forest, hale), pace(&server.forest, hale)),
        (Some("mare"), on_foot + 4.0),
        "into an area that allows the mount, onto ground it stands on: still riding, and what its script does holds on"
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
            told(&mut server.village, 2),
        ),
        (None, true, on_foot, Some(not_here.to_owned())),
        "into an area that allows the mount, onto a spot it can't stand on: on foot, and told why"
    );

    launch(&mut server, 2, hale);
    warp(&mut server.village, hale, "village", FAR_OUT);
    server.send(2, AreaDrawn);
    assert_eq!(
        (seat(&server, hale), at(&server, hale)),
        (Some("boat"), FAR_OUT),
        "within the area, onto water the boat floats on: still afloat"
    );
    warp(&mut server.village, hale, "village", landing);
    server.send(2, AreaDrawn);
    assert_eq!(
        (
            seat(&server, hale),
            walkable(&server, at(&server, hale)),
            at(&server, hale).distance(landing) < Meters(2.0),
            told(&mut server.village, 2),
        ),
        (
            None,
            true,
            true,
            Some("The Rowboat can't go where you arrived. You're on foot.".to_owned())
        ),
        "within the area, onto ground: on foot at the landing"
    );

    server.village.give(hale, "feather", 1);
    cast(&mut server, 2, "call_crow", CastAim::None);
    warp(&mut server.village, hale, "forest", landing);
    assert_eq!(
        (rides(&server.village, hale), told(&mut server.village, 2)),
        (None, Some("This area does not allow mounts".to_owned())),
        "into an area that bars the mount's kind: on foot before leaving, and told why"
    );
    server.village.hand_over(&mut server.forest);
    server.step();
    let hale = server.forest.player(2);
    assert_eq!(rides(&server.forest, hale), None, "and on foot there");
}

#[test]
fn a_character_saved_where_no_walker_stands_signs_in_where_dying_would_send_them() {
    let content = game().load();
    let village = content
        .by_path::<AreaDef>(&path("area", "village"))
        .expect("the village");
    let ashore = Pos::new(8.0, 6.0);
    for (saved, signs_in, case) in [
        (
            FAR_OUT,
            BEACH,
            "saved out on the lake, signs in at the spawn point",
        ),
        (
            ashore,
            ashore,
            "saved on the beach, signs in where they were",
        ),
    ] {
        let mut server = server();
        let snapshot = CharacterSnapshot {
            rises_at: Some(SnapshotSpot {
                x: BEACH.x,
                y: BEACH.y,
            }),
            ..CharacterSnapshot::of_def(&content, "Wren", player::def(&content))
        };
        let join_as = JoinAs {
            snapshot,
            start: Place {
                area: village,
                at: saved,
            },
            origin: JoinOrigin::Saved {
                at: server.now(),
                told: Vec::new(),
            },
        };
        let wren = server.sign_in_as(1, "Wren", "village", Some(join_as));
        server.step();
        assert!(
            at(&server, wren).distance(signs_in) < Meters(1.0),
            "{case}: at {:?}",
            at(&server, wren)
        );
    }
}

#[test]
fn the_game_answers_where_each_of_its_mounts_goes() {
    let content = game().engine_rows().load();
    let listed: ProbeGame = asked(&content, &ProbeGameQuestion::Game);
    let mut mounts: Vec<(String, MountKind, Vec<String>)> = listed
        .mounts
        .iter()
        .map(|mount| {
            let callers = mount
                .called_by
                .iter()
                .map(|skill| skill.get(&content).name.clone());
            (mount.name.clone(), mount.kind, callers.collect())
        })
        .collect();
    mounts.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        mounts,
        [
            (
                "Grey Mare".to_owned(),
                MountKind::Riding,
                vec!["call_mare".to_owned()]
            ),
            (
                "Rowboat".to_owned(),
                MountKind::Floating,
                vec!["call_boat".to_owned(), "call_raft".to_owned()]
            ),
            (
                "Stormcrow".to_owned(),
                MountKind::Flying,
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
                "rider": seat("body", "sit", json!(["items"])),
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
                "rider": seat("body", "sit", json!([])),
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
                "rider": seat("body", "row", json!(["gather", "items"])),
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
        .item("shell", casts("tide"))
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
    .chain([(
        json!({
            "name": "tide",
            "icon": "icons/skill.png",
            "target": "self",
            "levels": [{ "cost": 1, "stats": {} }],
            "modules": [{ "type": "script", "script": path("script", "tide_used") }],
        }),
        json!({
            "used": { "type": "on", "event": "used", "next": "off" },
            "off": { "type": "dismount" },
        }),
    )])
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

/// A server of `game` whose two worlds play what mounts take: walks, casts, items, and the scripts
/// and statuses a mount's own script runs through.
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
                engine::systems::roster::ListRoster::default(),
                engine::core::script::run::FlowRunner::default(),
                movement::AdvanceWalks::default(),
            ))
            .plan(mount::UnseatFallen::default())
            .plan(stat::SettleVitals::default())
            .add((
                resource::gathering,
                resource::work,
                combat::defeats,
                skill::hotbar::settle_basic_attack,
            ))
            .plan(actor::SettleActions::default());
        engine::systems::spawn_scripts(&mut sim.world);
    }
    server
}

impl Server {
    /// The last things the player was told, refusals among them.
    fn told_or_refused(&mut self, client: u32) -> Vec<String> {
        let sim = self.village_or_forest(client);
        sim.told(client)
            .into_iter()
            .map(|told| told.text.plain_words(sim.content()))
            .collect()
    }
}

/// The player of `client` launches the boat just off the beach, as `who`.
fn launch(server: &mut Server, client: u32, who: Entity) {
    stand(server, who, BEACH);
    server.village.give(who, "oar", 1);
    cast(server, client, "call_boat", CastAim::Point(OFFSHORE));
    assert_eq!(seat(server, who), Some("boat"), "a boat launched");
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

/// The mount `who` rides in the village.
fn seat(server: &Server, who: Entity) -> Option<&'static str> {
    rides(&server.village, who)
}

fn rides(sim: &Sim, who: Entity) -> Option<&'static str> {
    let riding = sim.world.get::<Riding>(who)?;
    ["mare", "crow", "boat"]
        .into_iter()
        .find(|mount| sim.id::<MountDef>(mount) == riding.mount)
}

fn walkable(server: &Server, at: Pos<Meters>) -> bool {
    let content = server.village.world.resource::<Content>();
    let village = server.village.id::<AreaDef>("village");
    area::load(content, village).mesh().walkable(at)
}
