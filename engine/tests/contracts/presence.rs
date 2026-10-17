use bevy_ecs::prelude::*;
use engine::core::script::Expr;
use engine::core::tiling::Tiles;
use engine::systems::combat::{AttackRequest, Attitude};
use engine::systems::item::{ItemId, ItemKind};
use engine::systems::movement::position;
use engine::systems::npc::{self, Pack};
use engine::systems::player::ClientId;
use engine::systems::reach;
use engine::systems::script::RiftQuery;
use engine::systems::script::condition::{self, CompiledCondition};
use engine::systems::stat::{self, StatKind};
use engine::systems::visibility::{self, Presence};

use testkit::{Sim, content, remember, row, spawn_area, townsperson};

#[test]
fn an_npc_spawned_for_one_player_cannot_be_fought_by_another() {
    let mut sim = Sim::area(spawn_area());
    let owner = sim.join(1);
    sim.join(2);
    let orc = orc_near(&mut sim, owner);
    sim.world()
        .entity_mut(orc)
        .insert(Presence::For(ClientId(1)));
    let full = stat::effective(sim.world(), orc, StatKind::Health);

    sim.send(2, AttackRequest { target: orc });
    sim.run_until(4.0, |_| false);
    assert_eq!(stat::effective(sim.world(), orc, StatKind::Health), full);

    sim.send(1, AttackRequest { target: orc });
    assert!(
        sim.run_until(10.0, |world| stat::effective(world, orc, StatKind::Health)
            < full)
    );
}

#[test]
fn a_friendly_npc_cannot_be_attacked() {
    let mut sim = Sim::area(spawn_area());
    let player = sim.join(1);
    let grisha = townsperson(&mut sim, row("Grisha"));
    assert_eq!(
        sim.world().get::<Attitude>(grisha),
        Some(&Attitude::Friendly)
    );
    let full = stat::effective(sim.world(), grisha, StatKind::Health);

    sim.send(1, AttackRequest { target: grisha });
    sim.run_until(4.0, |_| false);

    assert_eq!(reach::attacking(sim.world(), player), None);
    assert_eq!(stat::effective(sim.world(), grisha, StatKind::Health), full);
}

#[test]
fn a_monster_spawned_for_one_player_ignores_everyone_else() {
    let mut sim = Sim::area(spawn_area());
    let owner = sim.join(1);
    let bystander = sim.join(2);
    let start = position(sim.world(), bystander).expect("position");
    let spot = sim.walkable_near(start, Tiles(1.0), Tiles(2.0));
    let skeleton = npc::spawn(
        sim.world(),
        row("Skeleton"),
        spot,
        spawn_area(),
        Pack(u32::MAX),
    );
    sim.world()
        .entity_mut(skeleton)
        .insert(Presence::For(ClientId(1)));

    let mut hunted = Vec::new();
    sim.run_until(3.0, |world| {
        hunted.extend(reach::attacking(world, skeleton));
        false
    });

    assert!(hunted.contains(&owner));
    assert!(!hunted.contains(&bystander));
}

#[test]
fn presence_by_condition_follows_the_players_memory() {
    let pell_dead = Expr::Query(RiftQuery::Memory {
        of: None,
        memory: row("PellDead"),
    });
    let pell_alive = Expr::Not(Box::new(pell_dead));
    let mut sim = Sim::area(spawn_area());
    let player = sim.join(1);
    let orc = orc_near(&mut sim, player);
    sim.world()
        .entity_mut(orc)
        .insert(Presence::When(CompiledCondition::new(
            content(),
            &pell_alive,
        )));
    assert!(visibility::present(sim.world(), orc, player));

    remember(&mut sim, player, row("PellDead"));

    assert!(!visibility::present(sim.world(), orc, player));
}

#[test]
fn a_world_that_serves_no_area_judges_an_items_requirements() {
    let shield: ItemId = row("BoneShield");
    let ItemKind::Equipment {
        requires: Some(requirement),
        ..
    } = &shield.get(content()).kind
    else {
        panic!("the bone shield asks something of its wearer");
    };
    let mut world = World::new();
    world.insert_resource(content().clone());
    let viewer = world.spawn_empty().id();

    assert!(!condition::holds(&world, viewer, requirement));
}

fn orc_near(sim: &mut Sim, player: Entity) -> Entity {
    let start = position(sim.world(), player).expect("position");
    let spot = sim.walkable_near(start, Tiles(2.0), Tiles(4.0));
    npc::spawn(sim.world(), row("Orc"), spot, spawn_area(), Pack(u32::MAX))
}
