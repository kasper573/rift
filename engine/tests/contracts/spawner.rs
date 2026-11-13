use bevy_ecs::prelude::*;
use engine::core::math::{Pos, Rect, Size};
use engine::core::time::{DayClock, TimeCourse};
use engine::core::units::HourOfDay;
use engine::systems::movement::{self, position};
use engine::systems::npc::{self, Npc, NpcDef};
use engine::systems::spawner::{self, Spawned};
use engine::systems::{actor, area, combat, region, script, stat};
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
fn a_spawner_turns_with_the_sky_from_one_share_to_the_next_out_of_sight_and_over_its_wait() {
    const LONG: usize = 160;
    const SIGHT: f32 = 24.0;
    let during = |time: &str| json!({ "type": "during", "time": time });
    let mut sim = field(
        LONG,
        json!({
            "type": "spawner",
            "spawns": [
                { "type": "npc", "npc": path("npc", "boar"), "when": during("Day") },
                { "type": "npc", "npc": path("npc", "wolf"), "when": during("Night") },
            ],
            "count": COUNT,
            "respawn": WAIT,
        }),
        NOON,
    );
    let player = sim.join(1);
    let seen = |sim: &Sim, npc: Entity| {
        let from = position(&sim.world, player).expect("the player stands somewhere");
        position(&sim.world, npc).is_some_and(|at| at.distance_to(from) <= SIGHT)
    };
    let by_day = standing(&mut sim);
    assert_eq!(by_day.len(), COUNT);
    assert_eq!(kinds(&mut sim, &by_day), [false, true], "by day");
    let watched: Vec<Entity> = by_day
        .iter()
        .copied()
        .filter(|&boar| seen(&sim, boar))
        .collect();
    let boar = sim.id::<NpcDef>("boar");
    let boars = |sim: &mut Sim| -> Vec<Entity> {
        standing(sim)
            .into_iter()
            .filter(|&npc| sim.world.get::<Npc>(npc).is_some_and(|npc| npc.def == boar))
            .collect()
    };

    hold(&mut sim, MIDNIGHT);
    sim.run(0.5);
    assert_eq!(
        boars(&mut sim),
        watched,
        "at nightfall the boars nobody sees leave, and those in sight stay"
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
    for &npc in by_night.iter().filter(|npc| !watched.contains(npc)) {
        assert_eq!(kinds(&mut sim, &[npc]), [true, false], "by night");
        assert!(!seen(&sim, npc), "a wolf arrived in the player's sight");
    }

    movement::relocate(&mut sim.world, player, Pos::new(LONG as f32 - 2.0, 2.0));
    sim.run(WAIT + 1.0);
    let wolves = standing(&mut sim);
    assert_eq!(wolves.len(), COUNT);
    assert_eq!(
        kinds(&mut sim, &wolves),
        [true, false],
        "the boars leave once the player has gone"
    );

    hold(&mut sim, DUSK);
    sim.run(WAIT + 1.0);
    let lingering = standing(&mut sim);
    assert!(
        lingering.len() < COUNT && lingering.iter().all(|&wolf| seen(&sim, wolf)),
        "under a sky none of its shares holds a spawner stands empty out of sight"
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

/// A field whose one spawner keeps wolves and boars, three wolves to a boar, with `more` of its
/// fields.
fn wilds(more: Value) -> Sim {
    let mut pack = json!({
        "type": "spawner",
        "spawns": [
            { "type": "npc", "npc": path("npc", "wolf"), "weight": 3 },
            { "type": "npc", "npc": path("npc", "boar") },
        ],
        "count": COUNT,
        "within": { "x": 2.0, "y": 2.0, "w": 36.0, "h": 26.0 },
    });
    pack.as_object_mut()
        .expect("the spawner")
        .extend(more.as_object().expect("fields").clone());
    field(40, pack, NOON)
}

/// A field `width` metres wide of wolves and boars that `pack` keeps, its day held at `hour`.
fn field(width: usize, pack: Value, hour: f32) -> Sim {
    let time = |name: &str, from: f32| json!({ "name": name, "from": from, "eases": 0.0, "color": [0, 0, 0], "icon": "icons/time.png" });
    let content = Game::new()
        .area("wilds", &Map::open(width, 30), json!([pack]))
        .hero("wilds", 1.0, 1.0)
        .asset("icons/time.png", testkit::png(1, 1))
        .edit_settings(|settings| {
            settings["daylight"] = json!({
                "day_length": 1200.0,
                "start": NOON,
                "times": [time("Day", 6.0), time("Dusk", 18.0), time("Night", 21.0)],
            });
        })
        .npc("wolf", 20.0, json!([]))
        .npc("boar", 30.0, json!([]))
        .vfx("damage_taken")
        .shader("damage_pop")
        .vfx("defeated")
        .load();
    let mut sim = Sim::new(content);
    sim.serve("wilds")
        .plan(stat::SettleHealth::default())
        .add((
            area::weather::read_sky,
            combat::defeats,
            (npc::clear_fallen, spawner::retire, spawner::refill).chain(),
        ))
        .plan(actor::SettleActions::default());
    hold(&mut sim, hour);
    area::weather::read_sky(&mut sim.world);
    region::hold_whole(&mut sim.world);
    script::spawn_stateless(&mut sim.world);
    sim
}

/// Every living npc a spawner keeps, in the order they came to life.
fn standing(sim: &mut Sim) -> Vec<Entity> {
    let mut spawned = sim
        .world
        .query_filtered::<Entity, (With<Spawned>, With<Npc>)>();
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
