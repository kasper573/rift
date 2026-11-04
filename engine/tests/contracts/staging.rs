use engine::core::math::Pos;
use engine::core::time::{TimeCourse, WeatherClock};
use engine::core::units::{MeterPos, Meters};
use engine::systems::area::AreaDef;
use engine::systems::combat::{self, AttackRequest};
use engine::systems::interact::{self, InteractRequest};
use engine::systems::movement::{self, MoveRequest, position};
use engine::systems::npc::NpcDef;
use engine::systems::region;
use engine::systems::script::Place;
use engine::systems::staging::{Staging, stage};
use engine::systems::{actor, dialogue, npc, reach, script, skill, stat};
use serde_json::json;
use testkit::{Game, Map, Sim, hunting, path};

const SPAWN: Pos<Meters> = Pos::new(10.0, 4.0);

#[test]
fn a_bench_stages_what_a_test_character_does_through_its_own_requests() {
    let mut sim = sim();
    let player = sim.join(1);
    sim.equip(player, "sword");
    let conn = sim.conn(1);
    let keeper = sim.npc("keeper");
    let raider = sim.npc("raider");

    let walk_to = SPAWN + engine::core::math::Offset::new(0.0, 3.0);
    stage(&mut sim.world, conn, Staging::Walk { to: walk_to }).expect("a walk");
    assert!(sim.run_until(10.0, |world| {
        position(world, player).is_some_and(|at| at.distance(walk_to) < Meters(0.1))
    }));

    stage(&mut sim.world, conn, Staging::Talk).expect("someone to talk to");
    assert!(sim.run_until(10.0, |world| {
        world
            .get::<dialogue::Conversation>(player)
            .is_some_and(|talk| talk.with == Some(keeper))
    }));

    let area = sim.id::<AreaDef>("yard");
    let away = Pos::new(16.0, 7.0);
    stage(
        &mut sim.world,
        conn,
        Staging::Place {
            place: Place { area, at: away },
        },
    )
    .expect("a place");
    sim.tick();
    let placed = position(&sim.world, player).expect("a position");
    assert!(placed.distance(away) < Meters(1.5), "{placed:?}");
    assert!(sim.conversation(1).is_none());

    stage(&mut sim.world, conn, Staging::Fight).expect("a foe");
    assert!(sim.run_until(10.0, |world| reach::attacking(world, player)
        == Some(raider)));
}

#[test]
fn a_bench_is_told_when_there_is_nothing_to_stage() {
    let mut sim = Sim::new(
        Game::new()
            .area("yard", &Map::open(20, 9), json!([]))
            .hero("yard", SPAWN.x, SPAWN.y)
            .load(),
    );
    sim.serve("yard");
    sim.join(1);
    let conn = sim.conn(1);

    assert!(stage(&mut sim.world, conn, Staging::Fight).is_err());
    assert!(stage(&mut sim.world, conn, Staging::Talk).is_err());
}

#[test]
fn a_bench_holds_an_areas_weather_at_a_named_spell_until_staged_again() {
    let mut sim = Sim::new(
        Game::new()
            .area("yard", &Map::open(20, 9), json!([]))
            .edit("area", "yard", |area| {
                area["climate"] = json!({
                    "turns": 10.0,
                    "spells": [
                        {
                            "name": "fair",
                            "weather": { "clouds": 0.1 },
                            "lasts": { "shortest": 60.0, "longest": 60.0 },
                            "next": [{ "spell": "storm", "weight": 1.0 }],
                        },
                        {
                            "name": "storm",
                            "weather": { "clouds": 1.0, "water": 1.0, "rain": 1.0 },
                            "lasts": { "shortest": 60.0, "longest": 60.0 },
                            "next": [{ "spell": "fair", "weight": 1.0 }],
                        },
                    ],
                });
            })
            .hero("yard", SPAWN.x, SPAWN.y)
            .load(),
    );
    sim.serve("yard");
    sim.join(1);
    let conn = sim.conn(1);

    stage(
        &mut sim.world,
        conn,
        Staging::Weather {
            spell: "storm".to_owned(),
        },
    )
    .expect("a spell of the climate");
    assert_eq!(
        sim.world.resource::<WeatherClock>().0,
        Some(TimeCourse::Held(1))
    );
    sim.run_until(120.0, |_| false);
    assert_eq!(
        sim.world.resource::<WeatherClock>().0,
        Some(TimeCourse::Held(1)),
        "the spell holds however long the world runs"
    );
    assert!(
        stage(
            &mut sim.world,
            conn,
            Staging::Weather {
                spell: "monsoon".to_owned(),
            },
        )
        .is_err(),
        "a spell the climate lacks is refused"
    );
}

fn sim() -> Sim {
    let content = Game::new()
        .area(
            "yard",
            &Map::open(20, 9),
            json!([
                { "type": "resident", "npc": path("npc", "keeper"), "at": { "x": 10.0, "y": 1.0 } },
            ]),
        )
        .hero("yard", SPAWN.x, SPAWN.y)
        .sword()
        .npc(
            "keeper",
            40.0,
            json!([{ "type": "script", "script": path("script", "keeper_talk") }]),
        )
        .script(
            "keeper_talk",
            "npc",
            json!({
                "interacted": { "type": "on", "event": "interacted", "verb": "talk", "reach": 2.0, "next": "hello" },
                "hello": {
                    "type": "dialogue",
                    "lines": [{ "by": { "type": "npc", "npc": path("npc", "keeper"), "face": "neutral" }, "text": ["Welcome."] }],
                    "choices": [{ "label": ["Goodbye."] }],
                },
            }),
        )
        .bust_of("keeper")
        .npc(
            "raider",
            1_000.0,
            json!([{ "type": "script", "script": path("script", "raider_hunt") }]),
        )
        .row("script", "raider_hunt", hunting(1.0))
        .mark("talking")
        .sfx("ui_refuse")
        .sfx("ui_close")
        .vfx("damage_taken")
        .shader("damage_pop")
        .vfx("defeated")
        .load();
    let mut sim = Sim::new(content);
    sim.serve("yard")
        .accept::<MoveRequest>()
        .accept::<InteractRequest>()
        .accept::<AttackRequest>()
        .accept::<dialogue::ConversationRequest>()
        .add((
            movement::move_request,
            interact::interact_request,
            combat::request,
            dialogue::requests,
            actor::playing::progress,
        ))
        .plan(skill::casting::StepCasts::default())
        .plan((
            script::query::ListRoster::default(),
            npc::ApproachWatchers::default(),
            script::RunFlows::default(),
            npc::body::DriveBodies::default(),
            movement::AdvanceWalks::default(),
        ))
        .add((interact::interactions, dialogue::hold, dialogue::refresh))
        .plan(stat::SettleHealth::default())
        .add((combat::defeats, skill::hotbar::settle_basic_attack))
        .plan(actor::SettleActions::default());
    script::spawn_stateless(&mut sim.world);
    region::hold_whole(&mut sim.world);
    let raider = sim.id::<NpcDef>("raider");
    let area = sim.id::<AreaDef>("yard");
    npc::spawn(&mut sim.world, raider, Pos::new(18.0, 7.0), area, None);
    sim
}
