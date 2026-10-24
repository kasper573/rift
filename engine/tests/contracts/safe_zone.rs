use bevy_ecs::prelude::*;
use engine::core::math::Pos;
use engine::core::units::{Meters, TilePos};
use engine::systems::area::{self, AreaDef, Wild};
use engine::systems::combat::{self, AttackRequest};
use engine::systems::movement::{self, MoveRequest, MoveSeq, position};
use engine::systems::npc::{self, NpcDef};
use engine::systems::{actor, reach, script, skill, stat};
use serde_json::{Value, json};
use testkit::{Game, Map, Sim, hunting, path};

const LAIR: Pos<Meters> = Pos::new(1.0, 4.0);
const DOORSTEP: Pos<Meters> = Pos::new(4.0, 4.0);
const REFUGE: Pos<Meters> = Pos::new(8.0, 4.0);

#[test]
fn wild_monsters_never_set_foot_in_a_safe_zone() {
    let mut sim = border(json!([{ "type": "population", "npc": "wanderer.npc.json", "count": 6 }]));
    npc::spawn_all(&mut sim.world);
    let monsters: Vec<Entity> = sim
        .world
        .query_filtered::<Entity, With<Wild>>()
        .iter(&sim.world)
        .collect();
    assert_eq!(monsters.len(), 6);
    let spawns: Vec<_> = monsters
        .iter()
        .map(|&monster| position(&sim.world, monster))
        .collect();

    let mut trespassed = false;
    sim.run_until(30.0, |world| {
        trespassed |= monsters
            .iter()
            .any(|&monster| on_safe_ground(world, monster));
        false
    });

    let roamed = monsters
        .iter()
        .zip(&spawns)
        .filter(|&(&monster, &spawn)| position(&sim.world, monster) != spawn)
        .count();
    assert!(roamed > 0, "no monster roamed");
    assert!(!trespassed);
}

#[test]
fn a_wild_monster_gives_up_a_player_who_reaches_a_safe_zone() {
    let mut sim = border(json!([]));
    let player = sim.join(1);
    movement::relocate(&mut sim.world, player, DOORSTEP);
    let hunter = spawn(&mut sim, "hunter", LAIR);
    sim.world.entity_mut(hunter).insert(Wild);
    assert!(hunted_by(&mut sim, hunter, player));

    let conn = sim.conn(1);
    sim.send(
        conn,
        MoveRequest {
            pos: REFUGE,
            seq: MoveSeq(1),
        },
    );
    let mut trespassed = false;
    sim.run_until(10.0, |world| {
        trespassed |= on_safe_ground(world, hunter);
        false
    });

    assert!(on_safe_ground(&sim.world, player));
    assert!(!trespassed);
    assert_eq!(reach::attacking(&sim.world, hunter), None);
}

#[test]
fn a_summoned_monster_follows_its_target_into_a_safe_zone() {
    let mut sim = border(json!([]));
    let player = sim.join(1);
    movement::relocate(&mut sim.world, player, DOORSTEP);
    let hunter = sim.id::<NpcDef>("hunter");
    let guard = npc::summon(&mut sim.world, hunter, 1, player, None, Some(player))
        .first()
        .copied()
        .expect("summoned");
    assert!(hunted_by(&mut sim, guard, player));

    let conn = sim.conn(1);
    sim.send(
        conn,
        MoveRequest {
            pos: REFUGE,
            seq: MoveSeq(1),
        },
    );

    assert!(sim.run_until(10.0, |world| on_safe_ground(world, guard)));
    assert_eq!(reach::attacking(&sim.world, guard), Some(player));
}

#[test]
fn a_wild_flyer_flies_around_a_safe_zone_instead_of_over_it() {
    let mut sim = border(json!([]));
    let (from, to) = (DOORSTEP, Pos::new(15.0, 4.0));
    let content = sim
        .world
        .resource::<engine::core::content::Content>()
        .clone();
    let bat = sim.id::<NpcDef>("bat");
    let area = sim.id::<AreaDef>("border");
    let bat = npc::spawn_actor(&mut sim.world, bat.get(&content), from, area);
    sim.world.entity_mut(bat).insert(Wild);

    movement::goto(&mut sim.world, bat, to);
    let mut trespassed = false;
    let arrived = sim.run_until(60.0, |world| {
        trespassed |= on_safe_ground(world, bat);
        position(world, bat).is_some_and(|at| at.distance(to) < Meters(0.01))
    });

    assert!(arrived, "the bat never got there");
    assert!(!trespassed);
}

fn border(modules: Value) -> Sim {
    let mut modules = modules;
    modules.as_array_mut().expect("modules").push(json!({
        "type": "safe_zone",
        "rect": { "x": 6.5, "y": 1.5, "w": 6.0, "h": 5.0 },
    }));
    let content = Game::new()
        .area("border", &Map::open(20, 9), modules)
        .hero("border", 15.0, 4.0)
        .sword()
        .wearing(&["sword"])
        .npc(
            "wanderer",
            25.0,
            json!([{ "type": "script", "script": "wanderer_roam.script.json" }]),
        )
        .script(
            "wanderer_roam",
            "npc",
            json!({
                "spawned": { "type": "on", "event": "started", "next": "roam" },
                "roam": { "type": "wander", "priority": 1, "next": "rest" },
                "rest": { "type": "wait", "seconds": 0, "at_most": 1.0, "next": "roam" },
            }),
        )
        .npc(
            "hunter",
            1_000.0,
            json!([{ "type": "script", "script": "hunter_hunt.script.json" }]),
        )
        .row("script", "hunter_hunt", hunting(8.0))
        .edit("npc", "hunter", |hunter| {
            hunter["basic_attack"] = json!(path("skill", "strike"))
        })
        .npc("bat", 8.0, json!([]))
        .edit("model", "bat", |bat| {
            bat["modules"] = json!([{ "type": "airborne" }])
        })
        .vfx("damage_taken")
        .shader("damage_pop")
        .vfx("defeated")
        .load();
    let mut sim = Sim::new(content);
    sim.serve("border")
        .accept::<MoveRequest>()
        .accept::<AttackRequest>()
        .add((
            movement::move_request,
            combat::request,
            actor::playing::progress,
            skill::casting::casts,
        ))
        .add((
            script::query::take_tick_roster,
            script::run_tick,
            npc::body::drive,
            movement::advance,
        ))
        .add((
            stat::settle_health,
            combat::defeats,
            skill::hotbar::settle_basic_attack,
            actor::settle,
        ));
    script::spawn_stateless(&mut sim.world);
    sim
}

fn spawn(sim: &mut Sim, key: &str, at: Pos<Meters>) -> Entity {
    let area = sim.id::<AreaDef>("border");
    let def = sim.id::<NpcDef>(key);
    npc::spawn(&mut sim.world, def, at, area, npc::Pack(u32::MAX))
}

fn on_safe_ground(world: &World, entity: Entity) -> bool {
    match (area::of(world, entity), position(world, entity)) {
        (Some(area), Some(at)) => area.safe(at),
        _ => false,
    }
}

fn hunted_by(sim: &mut Sim, hunter: Entity, player: Entity) -> bool {
    sim.run_until(5.0, |world| reach::attacking(world, hunter) == Some(player))
}
