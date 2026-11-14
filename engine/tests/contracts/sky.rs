use bevy_ecs::prelude::*;
use engine::core::math::Pos;
use engine::core::time::{DayClock, TimeCourse, WallClock, WeatherClock};
use engine::core::units::HourOfDay;
use engine::systems::area::{self, AreaObject};
use engine::systems::dialogue::{self, ConversationRequest};
use engine::systems::interact::{self, InteractRequest};
use engine::systems::movement::{self, position};
use engine::systems::script::{self, SkyCondition, sky};
use engine::systems::status::{self, StatusEffectDef};
use engine::systems::visibility::present;
use engine::systems::{GameSettings, actor, npc, region, spawner};
use serde_json::{Value, json};
use testkit::{Game, Map, Sim, dialogue as says, lines, path};

#[test]
fn the_sky_reads_alike_whoever_asks_its_hour_its_times_of_day_and_its_weather() {
    let mut sim = hills(times());
    let night = json!({ "type": "time_share", "time": "Night" });
    let during = |time: &str| json!({ "type": "during", "time": time });
    let hour_is = |hour: f32| json!({ "type": "equal", "a": { "type": "hour" }, "b": hour });

    hold(&mut sim, 12.0);
    for (condition, share) in [
        (hour_is(12.0), 1.0),
        (hour_is(13.0), 0.0),
        (during("Day"), 1.0),
        (during("Night"), 0.0),
        (json!({ "type": "time_share", "time": "Day" }), 1.0),
        (night.clone(), 0.0),
        (json!({ "type": "hour" }), 1.0),
        (json!({ "type": "clouds" }), 0.7),
        (json!({ "type": "wind" }), 0.5),
        (
            json!({ "type": "at_least", "a": { "type": "rain" }, "b": 0.3 }),
            1.0,
        ),
    ] {
        assert_share(&sim, &condition, share, "at noon");
    }

    hold(&mut sim, 21.0);
    for (condition, share) in [
        (during("Night"), 1.0),
        (during("Dusk"), 0.0),
        (night.clone(), 0.5),
        (json!({ "type": "time_share", "time": "Dusk" }), 0.5),
        (
            json!({ "type": "subtract", "a": 1, "b": night.clone() }),
            0.5,
        ),
    ] {
        assert_share(&sim, &condition, share, "as night falls");
    }
    hold(&mut sim, 20.0);
    assert_share(&sim, &night, 0.0, "before night eases in");
    hold(&mut sim, 22.0);
    assert_share(&sim, &night, 1.0, "once night has eased in");

    let cycle = sim
        .world
        .resource::<GameSettings>()
        .daylight
        .clone()
        .expect("the hills have a day");
    let now = sim.world.resource::<WallClock>().now;
    sim.world
        .insert_resource(DayClock(Some(cycle.turning_from(HourOfDay(20.5), now))));
    sim.run(HOUR);
    assert_share(&sim, &night, 0.75, "an hour into a turning day");

    let mut plain = hills(json!([]));
    plain.tick();
    assert_share(
        &plain,
        &hour_is(12.0),
        1.0,
        "a game without a day stands at noon",
    );

    let content = sim
        .world
        .resource::<engine::core::content::Content>()
        .clone();
    for (condition, rule) in [
        (
            json!({ "type": "at_least", "a": { "type": "level" }, "b": 3 }),
            "reads a player",
        ),
        (during("Nite"), "unknown time of day"),
    ] {
        let fault = serde_json::from_value::<SkyCondition>(condition.clone())
            .expect("a condition")
            .check(&content)
            .expect_err("refused");
        assert_eq!(fault.rule, rule, "{condition}: {fault:?}");
    }
}

#[test]
fn a_script_hears_the_day_turn_out_of_one_time_of_day_and_into_the_next() {
    let mut sim = hills(times());
    let keeper = sim.npc("keeper");
    let lit = sim.id::<StatusEffectDef>("lit");

    hold(&mut sim, 22.0);
    sim.run(1.0);
    assert!(
        !status::has(&sim.world, keeper, lit),
        "a world that starts within a time heard it begin"
    );

    turn_from(&mut sim, 20.8);
    sim.run(1.0);
    assert!(
        !status::has(&sim.world, keeper, lit),
        "a day set to another hour was heard as a turn"
    );
    sim.run(HOUR * 0.3);
    assert!(
        status::has(&sim.world, keeper, lit),
        "night began unheard as the day turned into it"
    );

    turn_from(&mut sim, 4.9);
    sim.run(1.0);
    assert!(
        status::has(&sim.world, keeper, lit),
        "a day set to another hour was heard as a turn"
    );
    sim.run(HOUR * 0.2);
    assert!(
        !status::has(&sim.world, keeper, lit),
        "night ended unheard as the day turned out of it"
    );
}

#[test]
fn an_npc_is_away_for_the_times_of_day_its_absence_lists_but_for_whoever_is_in_the_middle_of_something_with_it()
 {
    let mut sim = hills(times());
    hold(&mut sim, 12.0);
    let customer = sim.join(1);
    let passer = sim.join(2);
    for player in [customer, passer] {
        movement::relocate(&mut sim.world, player, Pos::new(BAKERY.x - 1.0, BAKERY.y));
    }
    sim.run(0.5);
    let baker = sim.npc("baker");
    assert!(signs(&mut sim).is_empty(), "a sign stood by day");
    sim.talk(1, "baker");

    hold(&mut sim, 22.0);
    sim.run(0.5);
    assert_eq!(
        sim.npcs("baker"),
        [baker],
        "she left in the middle of a talk"
    );
    let sign = signs(&mut sim)[0];
    assert!(
        present(&sim.world, baker, customer) && !present(&sim.world, sign, customer),
        "to whoever talks with her she is still there, and her sign is not yet"
    );
    assert!(
        !present(&sim.world, baker, passer) && present(&sim.world, sign, passer),
        "to everyone else she is gone, and her sign stands"
    );
    assert_eq!(
        position(&sim.world, sign),
        Some(BAKERY),
        "the sign stands where she lives"
    );

    sim.leave(1);
    sim.run(0.5);
    assert!(
        sim.npcs("baker").is_empty(),
        "she stayed once the talk was over"
    );
    assert!(present(&sim.world, sign, customer));
    assert_eq!(
        lines(&sim.open_with(2, sign)),
        ["Closed. Back in the morning."],
        "reading the sign says her notice"
    );
    sim.leave(2);

    hold(&mut sim, 5.5);
    sim.run(0.5);
    assert!(
        sim.npcs("baker").is_empty() && signs(&mut sim).is_empty(),
        "a span without a notice leaves nothing"
    );

    hold(&mut sim, 12.0);
    sim.run(0.5);
    let back = sim.npc("baker");
    assert_eq!(position(&sim.world, back), Some(BAKERY));
    assert!(signs(&mut sim).is_empty(), "her sign outstayed her absence");
}

/// Where the baker lives in the hills.
const BAKERY: Pos<engine::core::units::Meters> = Pos::new(12.0, 4.0);

/// What stands in the hills as an away npc's sign.
fn signs(sim: &mut Sim) -> Vec<Entity> {
    let board = path("prop", "signboard");
    let mut objects = sim.world.query::<(Entity, &AreaObject)>();
    objects
        .iter(&sim.world)
        .filter(|(_, object)| object.look.as_deref() == Some(board.as_str()))
        .map(|(entity, _)| entity)
        .collect()
}

/// How many seconds an hour of the hills' day lasts.
const HOUR: f32 = 50.0;

fn times() -> Value {
    let time = |name: &str, from: f32, eases: f32| json!({ "name": name, "from": from, "eases": eases, "color": [0, 0, 0], "icon": "icons/time.png" });
    json!([
        time("Dawn", 5.0, 1.0),
        time("Day", 7.0, 1.0),
        time("Dusk", 19.0, 2.0),
        time("Night", 21.0, 2.0),
    ])
}

fn hold(sim: &mut Sim, hour: f32) {
    sim.world
        .insert_resource(DayClock(Some(TimeCourse::Held(HourOfDay(hour)))));
    sim.tick();
}

fn turn_from(sim: &mut Sim, hour: f32) {
    let cycle = sim
        .world
        .resource::<GameSettings>()
        .daylight
        .clone()
        .expect("the hills have a day");
    let now = sim.world.resource::<WallClock>().now;
    sim.world
        .insert_resource(DayClock(Some(cycle.turning_from(HourOfDay(hour), now))));
}

fn assert_share(sim: &Sim, condition: &Value, share: f32, when: &str) {
    let asked = serde_json::from_value::<SkyCondition>(condition.clone())
        .expect("a condition")
        .share_in(&sim.world);
    assert!(
        (asked - share).abs() < 0.01,
        "{when}, {condition} holds {asked}, not {share}"
    );
}

/// Hills under a windy, clouded, rainy sky whose day is divided into `times`, where a keeper
/// is lit for as long as a night that began as the day turned, and a baker whose script keeps
/// her at her counter is away at dawn and by night, when she leaves a sign.
fn hills(times: Value) -> Sim {
    let night = !times.as_array().expect("times").is_empty();
    let content = Game::new()
        .area(
            "hills",
            &Map::open(20, 9),
            json!([
                { "type": "resident", "npc": path("npc", "keeper"), "at": { "x": 6.0, "y": 4.0 } },
                { "type": "resident", "npc": path("npc", "baker"), "at": { "x": BAKERY.x, "y": BAKERY.y } },
            ]),
        )
        .npc(
            "baker",
            40.0,
            if night {
                json!([
                    { "type": "script", "script": path("script", "baker_talk") },
                    { "type": "absence", "spans": [
                        { "time": "Dawn" },
                        { "time": "Night", "notice": {
                            "text": ["Closed. Back in the morning."],
                            "sign": path("prop", "signboard"),
                        } },
                    ] },
                ])
            } else {
                json!([])
            },
        )
        .script(
            "baker_talk",
            "npc",
            json!({
                "interacted": { "type": "on", "event": "interacted", "verb": "talk", "reach": 3.0, "next": "greet" },
                "greet": says("Fresh bread.", json!([{ "label": ["Good day."] }])),
                "started": { "type": "on", "event": "started", "next": "rest" },
                "rest": { "type": "wait", "seconds": 0.1, "at_most": 0.1, "next": "still" },
                "still": { "type": "stop", "priority": 1, "next": "rest" },
            }),
        )
        .prop("signboard", json!({ "type": "none" }))
        .edit("area", "hills", |area| {
            area["climate"] = json!({
                "turns": 10.0,
                "spells": [{
                    "name": "squall",
                    "weather": {
                        "wind": { "direction": 240.0, "strength": 0.5 },
                        "clouds": 0.7,
                        "water": 1.0,
                        "rain": 0.4,
                    },
                    "lasts": { "shortest": 60.0, "longest": 60.0 },
                }],
            });
        })
        .hero("hills", 2.0, 2.0)
        .asset("icons/time.png", testkit::png(1, 1))
        .edit_settings(|settings| {
            settings["daylight"] = json!({ "day_length": HOUR * 24.0, "start": 10.0, "times": times });
        })
        .npc(
            "keeper",
            40.0,
            if night {
                json!([{ "type": "script", "script": path("script", "keeper_lamp") }])
            } else {
                json!([])
            },
        )
        .script(
            "keeper_lamp",
            "npc",
            if night {
                json!({
                    "fell": { "type": "on", "event": "time_began", "time": "Night", "next": "light" },
                    "light": {
                        "type": "apply_status",
                        "who": { "type": "owner" },
                        "effect": path("status_effect", "lit"),
                    },
                    "lifted": { "type": "on", "event": "time_ended", "time": "Night", "next": "douse" },
                    "douse": {
                        "type": "remove_status",
                        "who": { "type": "owner" },
                        "effect": path("status_effect", "lit"),
                    },
                })
            } else {
                json!({})
            },
        )
        .row(
            "status_effect",
            "lit",
            json!({ "name": "Lit", "icon": "icons/time.png", "category": "buff" }),
        )
        .shader("status_pop")
        .load();
    let mut sim = Sim::new(content);
    sim.serve("hills")
        .accept::<InteractRequest>()
        .accept::<ConversationRequest>()
        .add((
            area::weather::turn_sky,
            area::weather::read_sky,
            sky::raise_turns,
            status::expire,
            (npc::clear_fallen, spawner::retire, spawner::refill).chain(),
            interact::interact_request,
            dialogue::requests,
        ))
        .plan((
            status::aspect::SettleAspects::default(),
            status::aspect::TickAspects::default(),
        ))
        .plan((
            script::query::ListRoster::default(),
            script::RunFlows::default(),
            movement::AdvanceWalks::default(),
        ))
        .add((
            interact::interactions,
            (dialogue::hold, dialogue::refresh).chain(),
        ))
        .plan(actor::SettleActions::default());
    sim.world
        .insert_resource(WeatherClock(Some(TimeCourse::Held(0))));
    script::spawn_stateless(&mut sim.world);
    region::hold_whole(&mut sim.world);
    sim.later(HOUR * 24.0);
    sim
}
