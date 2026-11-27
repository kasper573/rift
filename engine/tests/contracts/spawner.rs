use bevy_ecs::entity_disabling::Disabled;
use bevy_ecs::prelude::*;
use bevy_ecs::query::Allow;
use engine::core::math::{Pos, Rect, Size};
use engine::core::time::{DayClock, TimeCourse};
use engine::core::units::{HourOfDay, Meters};
use engine::systems::movement::{self, position};
use engine::systems::npc::{self, Npc, NpcDef};
use engine::systems::spawner::{self, Spawned};
use engine::systems::{GameSettings, actor, area, combat, region, sleep, stat};
use serde_json::{Value, json};
use testkit::{Game, Map, Sim, path};

const COUNT: usize = 40;
const WAIT: f32 = 20.0;
const CORPSES_LINGER: f32 = 5.0;

#[test]
fn a_freed_entry_is_filled_again_after_the_spawners_wait_by_a_fresh_draw_from_its_mix() {
    let mut sim = wilds(json!({ "respawn": WAIT }));
    let first = standing(&mut sim);
    assert_eq!(first.len(), COUNT);
    assert_eq!(
        kinds(&mut sim, &first),
        [true, true],
        "the first fill drew one kind only"
    );

    let victim = first[0];
    stat::apply_damage(&mut sim.world, victim, 10_000.0);
    sim.run(CORPSES_LINGER + 0.5);
    assert_eq!(
        standing(&mut sim).len(),
        COUNT - 1,
        "the body still stands in its entry"
    );
    sim.run(WAIT - 1.0);
    assert_eq!(
        standing(&mut sim).len(),
        COUNT - 1,
        "the entry was filled before its wait was over"
    );
    sim.run(1.5);
    let refilled = standing(&mut sim);
    assert_eq!(refilled.len(), COUNT);
    assert!(!refilled.contains(&victim));

    for &npc in &refilled {
        stat::apply_damage(&mut sim.world, npc, 10_000.0);
    }
    sim.run(CORPSES_LINGER + WAIT + 1.0);
    let second = standing(&mut sim);
    assert_eq!(second.len(), COUNT);
    assert!(second.iter().all(|npc| !refilled.contains(npc)));
    assert_eq!(
        kinds(&mut sim, &second),
        [true, true],
        "the refill drew one kind only"
    );
    let within = Rect::new(Pos::new(2.0, 2.0), Size::new(36.0, 26.0));
    for &npc in &second {
        let at = position(&sim.world, npc).expect("it stands somewhere");
        assert!(within.contains(at), "{at:?} is outside the spawner's rect");
    }

    let mut once = wilds(json!({}));
    let only = standing(&mut once);
    stat::apply_damage(&mut once.world, only[0], 10_000.0);
    once.run(CORPSES_LINGER + 60.0);
    assert_eq!(
        standing(&mut once).len(),
        COUNT - 1,
        "a spawner with no wait filled an entry again"
    );
}

#[test]
fn a_spawner_turns_with_the_sky_from_one_share_to_the_next_over_its_wait_but_for_a_fight() {
    const LONG: usize = 160;
    const SIGHT: f32 = 24.0;
    let during = |time: &str| json!({ "type": "during", "time": time });
    let mut sim = field(
        LONG,
        json!({
            "type": "spawner",
            "populations": [{
                "spawns": [
                    { "type": "npc", "npc": path("npc", "boar"), "when": during("Day") },
                    { "type": "npc", "npc": path("npc", "wolf"), "when": during("Night") },
                ],
                "max_spawned_at_once": COUNT,
                "respawn": WAIT,
            }],
        }),
        NOON,
    );
    sim.add(sleep::settle).add(movement::file_by_chunk);
    let player = sim.join(1);
    let seen = |sim: &Sim, npc: Entity| {
        let from = position(&sim.world, player).expect("the player stands somewhere");
        position(&sim.world, npc).is_some_and(|at| at.distance_to(from) <= SIGHT)
    };
    let by_day = standing(&mut sim);
    assert_eq!(by_day.len(), COUNT);
    assert_eq!(kinds(&mut sim, &by_day), [false, true], "by day");
    let boar = sim.id::<NpcDef>("boar");
    let boars = |sim: &mut Sim| -> Vec<Entity> {
        standing(sim)
            .into_iter()
            .filter(|&npc| sim.world.get::<Npc>(npc).is_some_and(|npc| npc.def == boar))
            .collect()
    };
    let fighting = boars(&mut sim)[0];
    combat::inflict(&mut sim.world, fighting, Some(player), 1.0);

    hold(&mut sim, MIDNIGHT);
    sim.run(0.5);
    assert_eq!(
        boars(&mut sim),
        [fighting],
        "at nightfall every boar leaves, seen or not, but the one in a fight"
    );
    assert!(
        standing(&mut sim).len() < COUNT,
        "a spawner turned at a stroke as night fell"
    );
    sim.run(WAIT + 1.0);
    let by_night = standing(&mut sim);
    assert_eq!(
        by_night.len(),
        COUNT,
        "the freed entries fill over the wait"
    );
    for &npc in by_night.iter().filter(|&&npc| npc != fighting) {
        assert_eq!(kinds(&mut sim, &[npc]), [true, false], "by night");
        assert!(!seen(&sim, npc), "a wolf arrived in the player's sight");
    }

    let fight_lasts = sim.world.resource::<GameSettings>().claims_last.0;
    sim.run(fight_lasts + WAIT + 1.0);
    let wolves = standing(&mut sim);
    assert_eq!(wolves.len(), COUNT);
    assert_eq!(
        kinds(&mut sim, &wolves),
        [true, false],
        "the boar leaves once its fight is over"
    );

    hold(&mut sim, DUSK);
    sim.run(0.5);
    assert!(
        standing(&mut sim).is_empty(),
        "under a sky none of its shares holds a spawner stands empty"
    );
    hold(&mut sim, NOON);
    sim.run(0.5);
    assert!(
        standing(&mut sim).len() < COUNT,
        "a spawner filled at a stroke as its share came on"
    );
    sim.run(WAIT + 1.0);
    assert_eq!(standing(&mut sim).len(), COUNT);
}

const NOON: f32 = 12.0;
const DUSK: f32 = 19.0;
const MIDNIGHT: f32 = 0.0;

fn hold(sim: &mut Sim, hour: f32) {
    sim.world
        .insert_resource(DayClock(Some(TimeCourse::Held(HourOfDay(hour)))));
}

/// A field whose one spawner keeps one endless population of wolves and boars, three wolves to a
/// boar, with `more` of the population's fields.
fn wilds(more: Value) -> Sim {
    let mut population = json!({
        "spawns": [
            { "type": "npc", "npc": path("npc", "wolf"), "weight": 3 },
            { "type": "npc", "npc": path("npc", "boar") },
        ],
        "max_spawned_at_once": COUNT,
    });
    population
        .as_object_mut()
        .expect("the population")
        .extend(more.as_object().expect("fields").clone());
    let pack = json!({
        "type": "spawner",
        "populations": [population],
        "within": { "x": 2.0, "y": 2.0, "w": 36.0, "h": 26.0 },
    });
    field(40, pack, NOON)
}

/// A field `width` metres wide of wolves and boars that `pack` keeps, its day held at `hour`.
fn field(width: usize, pack: Value, hour: f32) -> Sim {
    let time = |name: &str, from: f32| json!({ "name": name, "from": from, "eases": 0.0, "color": [0, 0, 0], "icon": "icons/time.png" });
    let game = beasts(Game::new().area("wilds", &Map::open(width, 30), json!([pack])))
        .hero("wilds", 1.0, 1.0)
        .asset("icons/time.png", testkit::png(1, 1))
        .edit_settings(|settings| {
            settings["daylight"] = json!({
                "day_length": 1200.0,
                "start": NOON,
                "times": [time("Day", 6.0), time("Dusk", 18.0), time("Night", 21.0)],
            });
        });
    served(game, "wilds", hour)
}

/// Wolves, boars and hawks, and what their falls show.
fn beasts(game: Game) -> Game {
    game.npc("wolf", 20.0, json!([]))
        .npc("boar", 30.0, json!([]))
        .npc("hawk", 5.0, json!([]))
        .shader("damage_pop")
        .vfx("defeated")
}

/// The server of `area` in `game`, its spawners kept and its scripts running, its day held at
/// `hour`.
fn served(game: Game, area: &str, hour: f32) -> Sim {
    let mut sim = Sim::new(game.load());
    sim.serve(area)
        .plan(stat::SettleVitals::default())
        .add((
            area::weather::turn_sky,
            combat::defeats,
            (npc::clear_fallen, spawner::tend).chain(),
        ))
        .plan((
            actor::SettleActions::default(),
            engine::core::script::run::FlowRunner::default(),
        ));
    hold(&mut sim, hour);
    area::weather::turn_sky(&mut sim.world);
    region::hold_whole(&mut sim.world);
    engine::systems::spawn_scripts(&mut sim.world);
    sim
}

/// Every living npc a spawner keeps, asleep or awake, in the order they came to life.
fn standing(sim: &mut Sim) -> Vec<Entity> {
    let mut spawned = sim
        .world
        .query_filtered::<Entity, (With<Spawned>, With<Npc>, Allow<Disabled>)>();
    let mut all: Vec<Entity> = spawned.iter(&sim.world).collect();
    all.sort_unstable();
    all.into_iter()
        .filter(|&npc| !stat::is_dead(&sim.world, npc))
        .collect()
}

/// Whether there is a wolf, and a boar, among `npcs`.
fn kinds(sim: &mut Sim, npcs: &[Entity]) -> [bool; 2] {
    ["wolf", "boar"].map(|kind| {
        let def = sim.id::<NpcDef>(kind);
        npcs.iter()
            .any(|&npc| sim.world.get::<Npc>(npc).is_some_and(|npc| npc.def == def))
    })
}

#[test]
fn an_entry_due_where_nobody_is_near_waits_until_a_player_brings_its_ground_back() {
    const RESPAWN: f32 = 10.0;
    const HOME: Pos<Meters> = Pos::new(550.0, 6.0);
    let time = |name: &str, from: f32| json!({ "name": name, "from": from, "eases": 0.0, "color": [0, 0, 0], "icon": "icons/time.png" });
    let content = Game::new()
        .npc("hawk", 5.0, json!([]))
        .npc(
            "keeper",
            40.0,
            json!([{ "type": "absence", "spans": [{ "time": "Night" }] }]),
        )
        .area(
            "field",
            &Map::open(600, 12),
            json!([
                {
                    "type": "spawner",
                    "populations": [{
                        "spawns": [{ "type": "npc", "npc": path("npc", "hawk") }],
                        "max_spawned_at_once": 1,
                        "respawn": RESPAWN,
                    }],
                    "within": { "x": 540.0, "y": 2.0, "w": 20.0, "h": 8.0 },
                },
                { "type": "resident", "npc": path("npc", "keeper"), "at": { "x": HOME.x, "y": HOME.y } },
            ]),
        )
        .hero("field", 4.0, 6.0)
        .asset("icons/time.png", testkit::png(1, 1))
        .edit_settings(|settings| {
            settings["daylight"] = json!({
                "day_length": 1200.0,
                "start": NOON,
                "times": [time("Day", 6.0), time("Night", 21.0)],
            });
        })
        .load_to_play();
    let mut sim = Sim::new(content.clone());
    let field = area::load(&content, sim.id::<area::AreaDef>("field"));
    sim.serve("field").add(
        (
            area::weather::turn_sky,
            sleep::settle,
            region::settle,
            spawner::tend,
        )
            .chain(),
    );
    hold(&mut sim, NOON);
    let hero = sim.join(1);
    // The ground is held on a thread of its own, which takes real time, not the world's.
    let go_to = |sim: &mut Sim, x: f32, until: &dyn Fn(&mut Sim) -> bool| {
        sim.world
            .entity_mut(hero)
            .insert(movement::Position::at(Pos::new(x, 6.0)));
        let mut waited = 0;
        while !until(sim) && waited < 1000 {
            std::thread::sleep(std::time::Duration::from_millis(5));
            sim.tick();
            waited += 1;
        }
    };

    go_to(&mut sim, 545.0, &|sim| standing(sim).len() == 2);
    let hawk = sim.npc("hawk");
    go_to(&mut sim, 4.0, &|_| !field.mesh().walkable(HOME));
    sim.world.despawn(hawk);
    hold(&mut sim, MIDNIGHT);
    sim.later(RESPAWN + 1.0);
    hold(&mut sim, NOON);
    sim.later(1.0);
    assert!(
        standing(&mut sim).is_empty(),
        "an entry due on ground nobody holds stays empty: a spawner's, and that of a resident \
         back from its hours away"
    );
    go_to(&mut sim, 545.0, &|sim| standing(sim).len() == 2);
    let keeper = sim.npc("keeper");
    assert_eq!(
        (sim.npcs("hawk").len(), position(&sim.world, keeper)),
        (1, Some(HOME)),
        "each entry fills once a player brings its ground back, the resident at its home"
    );
}

#[test]
fn a_spawners_populations_take_their_turns_one_after_another_and_its_area_hears_of_each() {
    const DELAY: f32 = 8.0;
    let wave = |npc: &str, size: u32| json!({ "spawns": [{ "type": "npc", "npc": path("npc", npc) }], "size": size, "max_spawned_at_once": size });
    let game = |more: Value| {
        let mut waves = json!({
            "type": "spawner",
            "name": "waves",
            "delay_between": DELAY,
            "populations": [wave("wolf", 2), wave("boar", 1)],
        });
        waves
            .as_object_mut()
            .expect("the spawner")
            .extend(more.as_object().expect("fields").clone());
        beasts(Game::new().area(
            "vault",
            &Map::open(20, 20),
            json!([
                { "type": "script", "script": "vault_waves.script.json" },
                waves,
                {
                    "type": "spawner",
                    "name": "reward",
                    "dormant": true,
                    "populations": [wave("hawk", 1)],
                },
            ]),
        ))
        .script(
            "vault_waves",
            "area",
            json!({
                "done": { "type": "on", "event": "spawner_exhausted", "spawner": "waves", "next": "reward" },
                "reward": { "type": "wake_spawner", "spawner": "reward" },
            }),
        )
        .hero("vault", 1.0, 1.0)
    };
    let fell = |sim: &mut Sim| {
        for npc in standing(sim) {
            stat::apply_damage(&mut sim.world, npc, 10_000.0);
        }
        sim.run(0.5);
    };

    let mut sim = served(game(json!({ "dormant": true })), "vault", NOON);
    sim.run(2.0);
    assert_eq!(named(&mut sim), [""; 0], "a dormant spawner spawns nothing");
    spawner::wake(&mut sim.world, "waves");
    sim.run(0.5);
    assert_eq!(
        named(&mut sim),
        ["wolf", "wolf"],
        "woken, its first population takes its turn"
    );
    fell(&mut sim);
    sim.run(DELAY - 1.0);
    assert_eq!(
        named(&mut sim),
        [""; 0],
        "the next waits out the delay between them"
    );
    sim.run(1.5);
    assert_eq!(
        named(&mut sim),
        ["boar"],
        "then the next population takes its turn"
    );
    fell(&mut sim);
    sim.run(0.5);
    assert_eq!(
        named(&mut sim),
        ["hawk"],
        "the area's script heard the spawner exhausted and woke the next one"
    );
    sim.run(DELAY * 3.0);
    assert_eq!(
        named(&mut sim),
        ["hawk"],
        "a spawner that does not repopulate is spent"
    );

    let mut sim = served(game(json!({ "repopulate": true })), "vault", NOON);
    sim.run(0.5);
    assert_eq!(
        named(&mut sim),
        ["wolf", "wolf"],
        "a spawner awake from the start begins at once"
    );
    fell(&mut sim);
    sim.run(DELAY + 0.5);
    fell(&mut sim);
    sim.run(DELAY + 0.5);
    assert_eq!(
        named(&mut sim),
        ["wolf", "wolf"],
        "a spawner that repopulates goes round its populations again"
    );
}

/// The kind of every living npc a spawner keeps, in order.
fn named(sim: &mut Sim) -> Vec<&'static str> {
    let npcs = standing(sim);
    let mut names: Vec<&'static str> = npcs
        .iter()
        .filter_map(|&npc| {
            let def = sim.world.get::<Npc>(npc)?.def;
            ["wolf", "boar", "hawk"]
                .into_iter()
                .find(|kind| sim.id::<NpcDef>(kind) == def)
        })
        .collect();
    names.sort_unstable();
    names
}
