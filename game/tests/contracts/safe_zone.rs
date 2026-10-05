use bevy_ecs::prelude::*;
use game::core::math::Pos;
use game::core::tiling::{TilePos, Tiles};
use game::data;
use game::data::npc::Id as NpcId;
use game::systems::area::{self, Area, Wild};
use game::systems::movement::{self, MoveRequest, Position, position};
use game::systems::npc::{self, Pack, ShownFor, SpawnNear, SpawnNpcs, Summoned};
use game::systems::reach::{self, ReachAct};
use game::systems::rule::{Encounter, Terms};

use crate::support::Sim;

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

fn place(sim: &mut Sim, entity: Entity, at: Pos<Tiles>) {
    sim.world().entity_mut(entity).insert(Position { pos: at });
}

fn wild(sim: &mut Sim, what: NpcId, at: Pos<Tiles>) -> Entity {
    let monster = npc::spawn(sim.world(), what, at, data::area::SPAWN_ID, Pack(u32::MAX));
    sim.world().entity_mut(monster).insert(Wild);
    monster
}

fn summon(sim: &mut Sim, player: Entity, what: NpcId) -> Entity {
    let spawn = SpawnNpcs {
        npc: what,
        count: 1,
        near: SpawnNear::Player,
        shown: ShownFor::Everyone,
    };
    let terms = Terms {
        requires: &[],
        costs: &[],
        outcomes: &[&spawn],
    };
    terms
        .settle(sim.world(), player, Encounter::default())
        .expect("summoned");
    let world = sim.world();
    world
        .query_filtered::<Entity, With<Summoned>>()
        .single(world)
        .expect("one summon")
}

fn hunted_by(sim: &mut Sim, hunter: Entity, player: Entity) -> bool {
    sim.run_until(5.0, |world| {
        reach::intent(world, hunter, ReachAct::Attack) == Some(player)
    })
}

#[test]
fn wild_monsters_never_set_foot_in_a_safe_zone() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
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
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let edge = edge(sim.map());
    let player = sim.join(1);
    place(&mut sim, player, edge.doorstep);
    let skeleton = wild(&mut sim, NpcId::Skeleton, edge.lair);
    assert!(hunted_by(&mut sim, skeleton, player));

    sim.send(1, MoveRequest { pos: edge.refuge });
    let mut trespassed = false;
    sim.run_until(10.0, |world| {
        trespassed |= on_safe_ground(world, skeleton);
        false
    });

    assert!(on_safe_ground(sim.world(), player));
    assert!(!trespassed);
    assert_eq!(reach::intent(sim.world(), skeleton, ReachAct::Attack), None);
}

#[test]
fn a_summoned_monster_follows_its_target_into_a_safe_zone() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let edge = edge(sim.map());
    let player = sim.join(1);
    place(&mut sim, player, edge.doorstep);
    let guard = summon(&mut sim, player, NpcId::Skeleton);
    assert!(hunted_by(&mut sim, guard, player));

    sim.send(1, MoveRequest { pos: edge.refuge });

    assert!(sim.run_until(10.0, |world| on_safe_ground(world, guard)));
    assert_eq!(
        reach::intent(sim.world(), guard, ReachAct::Attack),
        Some(player)
    );
}

#[test]
fn a_wild_flyer_flies_around_a_safe_zone_instead_of_over_it() {
    let mut sim = Sim::area(data::area::SPAWN_ID);
    let map = sim.map();
    let ground = map.wild_grid.nodes();
    let (from, to) = ground
        .iter()
        .flat_map(|&a| ground.iter().map(move |&b| (a, b)))
        .filter(|&(a, b)| map.safe(a.lerp(b, 0.5)))
        .min_by(|(a, b), (c, d)| a.distance(*b).0.total_cmp(&c.distance(*d).0))
        .expect("two points whose straight line crosses a safe zone");
    let bat = wild(&mut sim, NpcId::Bat, from);

    movement::goto(sim.world(), bat, to);
    let mut trespassed = false;
    let arrived = sim.run_until(60.0, |world| {
        trespassed |= on_safe_ground(world, bat);
        position(world, bat).is_some_and(|at| at.distance(to) < Tiles(0.01))
    });

    assert!(arrived, "the bat never got there");
    assert!(!trespassed);
}
