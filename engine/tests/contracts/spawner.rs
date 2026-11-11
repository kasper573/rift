use bevy_ecs::prelude::*;
use engine::core::math::{Pos, Rect, Size};
use engine::systems::movement::position;
use engine::systems::npc::{self, Npc, NpcDef};
use engine::systems::spawner::{self, Spawned};
use engine::systems::{actor, combat, region, script, stat};
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
    let content = Game::new()
        .area("wilds", &Map::open(40, 30), json!([pack]))
        .hero("wilds", 1.0, 1.0)
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
            combat::defeats,
            (npc::clear_fallen, spawner::refill).chain(),
        ))
        .plan(actor::SettleActions::default());
    region::hold_whole(&mut sim.world);
    script::spawn_stateless(&mut sim.world);
    sim
}

/// Every living npc a spawner keeps.
fn standing(sim: &mut Sim) -> Vec<Entity> {
    let mut spawned = sim
        .world
        .query_filtered::<Entity, (With<Spawned>, With<Npc>)>();
    let all: Vec<Entity> = spawned.iter(&sim.world).collect();
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
