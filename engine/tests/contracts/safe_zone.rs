use bevy_ecs::prelude::*;
use engine::core::math::Pos;
use engine::core::tiling::{TilePos, Tiles};
use engine::systems::area::{self, Area, Wild};
use engine::systems::movement::{self, MoveRequest, MoveSeq, position};
use engine::systems::npc::{self, NpcId, Pack};
use engine::systems::reach;

use testkit::{Sim, content, place, row, spawn_area};

#[test]
fn wild_monsters_never_set_foot_in_a_safe_zone() {
    let mut sim = Sim::area(spawn_area());
    sim.join(1);
    let world = sim.world();
    let monsters: Vec<Entity> = world
        .query_filtered::<Entity, With<Wild>>()
        .iter(world)
        .collect();
    assert!(!monsters.is_empty(), "the island has wild monsters");

    let mut trespassed = false;
    sim.run_until(30.0, |world| {
        trespassed |= monsters
            .iter()
            .any(|&monster| on_safe_ground(world, monster));
        false
    });

    assert!(!trespassed);
}

#[test]
fn a_wild_monster_gives_up_a_player_who_reaches_a_safe_zone() {
    let mut sim = Sim::area(spawn_area());
    let edge = edge(sim.map());
    let player = sim.join(1);
    place(&mut sim, player, edge.doorstep);
    let skeleton = wild(&mut sim, row("Skeleton"), edge.lair);
    assert!(hunted_by(&mut sim, skeleton, player));

    sim.send(
        1,
        MoveRequest {
            pos: edge.refuge,
            seq: MoveSeq(1),
        },
    );
    let mut trespassed = false;
    sim.run_until(10.0, |world| {
        trespassed |= on_safe_ground(world, skeleton);
        false
    });

    assert!(on_safe_ground(sim.world(), player));
    assert!(!trespassed);
    assert_eq!(reach::attacking(sim.world(), skeleton), None);
}

#[test]
fn a_summoned_monster_follows_its_target_into_a_safe_zone() {
    let mut sim = Sim::area(spawn_area());
    let edge = edge(sim.map());
    let player = sim.join(1);
    place(&mut sim, player, edge.doorstep);
    let guard = summon(&mut sim, player, row("Skeleton"));
    assert!(hunted_by(&mut sim, guard, player));

    sim.send(
        1,
        MoveRequest {
            pos: edge.refuge,
            seq: MoveSeq(1),
        },
    );

    assert!(sim.run_until(10.0, |world| on_safe_ground(world, guard)));
    assert_eq!(reach::attacking(sim.world(), guard), Some(player));
}

#[test]
fn a_wild_flyer_flies_around_a_safe_zone_instead_of_over_it() {
    let mut sim = Sim::area(spawn_area());
    let map = sim.map();
    let ground = map.wild_grid.nodes();
    let (from, to) = ground
        .iter()
        .flat_map(|&a| ground.iter().map(move |&b| (a, b)))
        .filter(|&(a, b)| map.safe(a.lerp(b, 0.5)))
        .min_by(|(a, b), (c, d)| a.distance(*b).0.total_cmp(&c.distance(*d).0))
        .expect("two points whose straight line crosses a safe zone");
    let bat = unscripted_wild(&mut sim, row("Bat"), from);

    movement::goto(sim.world(), bat, to);
    let mut trespassed = false;
    let arrived = sim.run_until(60.0, |world| {
        trespassed |= on_safe_ground(world, bat);
        position(world, bat).is_some_and(|at| at.distance(to) < Tiles(0.01))
    });

    assert!(arrived, "the bat never got there");
    assert!(!trespassed);
}

struct Edge {
    lair: Pos<Tiles>,
    doorstep: Pos<Tiles>,
    refuge: Pos<Tiles>,
}

fn edge(map: &Area) -> Edge {
    let ground = &map.grid;
    let outside = &map.wild_grid;
    outside
        .nodes()
        .iter()
        .find_map(|&doorstep| {
            let refuge = ground.nodes().iter().copied().find(|&inside| {
                map.safe(inside)
                    && (3.0..=4.5).contains(&doorstep.distance(inside).0)
                    && ground.component(inside) == ground.component(doorstep)
            })?;
            let lair = outside.nodes().iter().copied().find(|&out| {
                (2.5..=3.5).contains(&out.distance(doorstep).0)
                    && out.distance(refuge).0 > doorstep.distance(refuge).0 + 2.0
                    && outside.component(out) == outside.component(doorstep)
            })?;
            Some(Edge {
                lair,
                doorstep,
                refuge,
            })
        })
        .expect("a safe zone with open ground beside it")
}

fn on_safe_ground(world: &World, entity: Entity) -> bool {
    match (area::of(world, entity), position(world, entity)) {
        (Some(area), Some(at)) => area.safe(at),
        _ => false,
    }
}

fn wild(sim: &mut Sim, what: NpcId, at: Pos<Tiles>) -> Entity {
    let monster = npc::spawn(sim.world(), what, at, spawn_area(), Pack(u32::MAX));
    sim.world().entity_mut(monster).insert(Wild);
    monster
}

fn unscripted_wild(sim: &mut Sim, what: NpcId, at: Pos<Tiles>) -> Entity {
    let monster = npc::spawn_actor(sim.world(), what.get(content()), at, spawn_area());
    sim.world().entity_mut(monster).insert(Wild);
    monster
}

fn summon(sim: &mut Sim, player: Entity, what: NpcId) -> Entity {
    npc::summon(sim.world(), what, 1, player, None, Some(player))
        .first()
        .copied()
        .expect("summoned")
}

fn hunted_by(sim: &mut Sim, hunter: Entity, player: Entity) -> bool {
    sim.run_until(5.0, |world| reach::attacking(world, hunter) == Some(player))
}
