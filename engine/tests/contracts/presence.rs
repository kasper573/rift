use bevy_ecs::prelude::*;
use engine::core::math::{Offset, Pos};
use engine::core::script::Value;
use engine::core::units::Meters;
use engine::systems::area::AreaDef;
use engine::systems::combat::{self, AttackRequest};
use engine::systems::item::{ItemDef, ItemKind};
use engine::systems::memory::{self, MemoryDef};
use engine::systems::movement::{self, position};
use engine::systems::npc::{self, NpcDef, Pack};
use engine::systems::player::ClientId;
use engine::systems::script;
use engine::systems::script::condition;
use engine::systems::stat::{self, StatKind};
use engine::systems::visibility;
use engine::systems::{actor, reach, skill};
use serde_json::json;
use testkit::{Declared, Game, Map, Sim, hunting, path};

#[test]
fn a_monster_summoned_for_one_player_fights_only_that_player() {
    let mut sim = arena();
    let owner = sim.join(1);
    let bystander = sim.join(2);
    let hunter = sim.id::<NpcDef>("hunter");
    let hunter = npc::summon(
        &mut sim.world,
        hunter,
        1,
        bystander,
        Some(ClientId(1)),
        None,
    )[0];
    let full = stat::effective(&sim.world, hunter, StatKind::Health);

    let other = sim.conn(2);
    sim.send(other, AttackRequest { target: hunter });
    let mut hunted = Vec::new();
    sim.run_until(4.0, |world| {
        hunted.extend(reach::attacking(world, hunter));
        false
    });
    assert!(hunted.contains(&owner));
    assert!(!hunted.contains(&bystander));
    assert_eq!(stat::effective(&sim.world, hunter, StatKind::Health), full);

    let own = sim.conn(1);
    sim.send(own, AttackRequest { target: hunter });
    assert!(sim.run_until(10.0, |world| stat::effective(
        world,
        hunter,
        StatKind::Health
    ) < full));
}

#[test]
fn a_friendly_npc_cannot_be_attacked() {
    let mut sim = arena();
    let player = sim.join(1);
    let friend = spawn(&mut sim, "brute", player, Offset::new(1.0, 0.0));
    let full = stat::effective(&sim.world, friend, StatKind::Health);

    let conn = sim.conn(1);
    sim.send(conn, AttackRequest { target: friend });
    sim.run(4.0);

    assert_eq!(reach::attacking(&sim.world, player), None);
    assert_eq!(stat::effective(&sim.world, friend, StatKind::Health), full);
}

#[test]
fn a_resident_shown_by_condition_follows_the_players_memory() {
    let mut sim = arena();
    let player = sim.join(1);
    npc::spawn_all(&mut sim.world);
    let ghost = sim.npc("ghost");
    assert!(visibility::present(&sim.world, ghost, player));

    let slain = sim.id::<MemoryDef>("slain");
    memory::set(&mut sim.world, player, slain, Value::Bool(true));

    assert!(!visibility::present(&sim.world, ghost, player));
}

#[test]
fn a_world_that_serves_no_area_judges_an_items_requirements() {
    let content = Game::new()
        .item(
            "shield",
            json!({
                "kind": {
                    "type": "equipment",
                    "slot": "offhand",
                    "requires": { "type": "at_least", "a": { "type": "level" }, "b": 2 },
                },
            }),
        )
        .load();
    let shield = content.declared::<ItemDef>("shield").expect("the shield");
    let ItemKind::Equipment {
        requires: Some(requirement),
        ..
    } = &shield.get(&content).kind
    else {
        panic!("the shield asks something of its wearer");
    };
    let mut world = World::new();
    world.insert_resource(content.clone());
    let viewer = world.spawn_empty().id();

    assert!(!condition::holds(&world, viewer, requirement));
}

fn arena() -> Sim {
    let content = Game::new()
        .area(
            "arena",
            &Map::open(11, 7),
            json!([{
                "type": "resident",
                "npc": "ghost.npc.json",
                "at": { "x": 8.0, "y": 5.0 },
                "shown": { "type": "not", "of": { "type": "memory", "memory": "slain.memory.json" } },
            }]),
        )
        .hero("arena", 3.0, 3.0)
        .sword()
        .wearing(&["sword"])
        .npc("brute", 25.0, json!([]))
        .npc("ghost", 25.0, json!([]))
        .npc(
            "hunter",
            25.0,
            json!([{ "type": "script", "script": "hunter_hunt.script.json" }]),
        )
        .row("script", "hunter_hunt", hunting(8.0))
        .edit("npc", "hunter", |hunter| hunter["basic_attack"] = json!(path("skill", "strike")))
        .memory("slain", "never")
        .vfx("damage_taken")
        .shader("damage_pop")
        .vfx("defeated")
        .load();
    let mut sim = Sim::new(content);
    sim.serve("arena")
        .accept::<AttackRequest>()
        .add((
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

fn spawn(sim: &mut Sim, key: &str, near: Entity, offset: Offset<Meters>) -> Entity {
    let at: Pos<Meters> = position(&sim.world, near).expect("a position") + offset;
    let area = sim.id::<AreaDef>("arena");
    let def = sim.id::<NpcDef>(key);
    npc::spawn(&mut sim.world, def, at, area, Pack(u32::MAX))
}
