use bevy_ecs::prelude::*;
use bevy_time::Time;
use engine::core::math::Pos;
use engine::core::time::WallClock;
use engine::systems::actor::Actor;
use engine::systems::area::{AreaDef, Cell};
use engine::systems::combat;
use engine::systems::handoff::Moved;
use engine::systems::identity::{self, GlobalId, Named};
use engine::systems::mirror::{self, Ghost};
use engine::systems::movement::{self, MoveRequest, MoveSeq, Position, position};
use engine::systems::npc::{self, Npc, NpcDef};
use engine::systems::player::{ArrivalHold, ClientId, Players};
use engine::systems::stat::{self, Stats};
use engine::systems::worlds::{self, Worlds};
use serde_json::json;
use testkit::{Game, Map, Sim};

/// An area wider than a cell runs as one world per cell. What one world owns near a border, the
/// world next door holds as a ghost standing where its owner does, gone once it leaves the halo;
/// a blow dealt to the ghost lands on its owner a tick later.
#[test]
fn an_actor_near_a_border_is_a_ghost_next_door_and_a_blow_on_the_ghost_lands_on_it() {
    let content = Game::new()
        .area("plain", &Map::open(500, 250), json!([]))
        .hero("plain", 150.0, 150.0)
        .npc("brute", 25.0, json!([]))
        .vfx("damage_taken")
        .shader("damage_pop")
        .load();
    let worlds = Worlds::of(&content);
    assert_eq!(worlds.len(), 2, "a 500 m strip is two cells of 250 m");

    let mut west = Sim::new(content.clone());
    let mut east = Sim::new(content);
    for (sim, cell) in [(&mut west, Cell(0)), (&mut east, Cell(1))] {
        sim.serve("plain")
            .in_cell(cell)
            .mirror::<Position>()
            .mirror::<Actor>()
            .mirror::<Stats>()
            .mirror::<Npc>()
            .add(
                (
                    identity::name_replicated,
                    mirror::deliver,
                    stat::settle_health,
                )
                    .chain(),
            );
    }
    west.join(1);
    let area = east.id::<AreaDef>("plain");
    let brute = east.id::<NpcDef>("brute");
    let at = Pos::new(270.0, 150.0);
    let brute = npc::spawn(&mut east.world, brute, at, area, None);

    tick(&worlds, &mut west, &mut east);

    let name = east
        .world
        .get::<GlobalId>(brute)
        .copied()
        .expect("the brute is named");
    let ghost = ghost_of(&west.world, name).expect("a ghost of the brute in the west");
    assert_eq!(position(&west.world, ghost), Some(at));

    combat::inflict(&mut west.world, ghost, None, 10.0);
    tick(&worlds, &mut west, &mut east);
    tick(&worlds, &mut west, &mut east);
    assert_eq!(stat::current_health(&east.world, brute), 15.0);
    assert_eq!(
        stat::current_health(&west.world, ghost),
        15.0,
        "the ghost shows the blow"
    );

    east.world.get_mut::<Position>(brute).unwrap().pos = Pos::new(295.0, 150.0);
    tick(&worlds, &mut west, &mut east);
    assert!(
        ghost_of(&west.world, name).is_none(),
        "out of the halo, the ghost goes"
    );
}

/// An actor that walks deep into the next cell changes hands: the cell's world takes it up
/// under its name, as it stands, and the world it came from lets it go; a player's connection
/// is told to follow.
#[test]
fn an_actor_deep_in_the_next_cell_changes_hands_as_it_stands() {
    let content = Game::new()
        .area("plain", &Map::open(500, 250), json!([]))
        .hero("plain", 280.0, 150.0)
        .npc("brute", 25.0, json!([]))
        .vfx("damage_taken")
        .shader("damage_pop")
        .load();
    let worlds = Worlds::of(&content);
    let mut west = Sim::new(content.clone());
    let mut east = Sim::new(content);
    for (sim, cell) in [(&mut west, Cell(0)), (&mut east, Cell(1))] {
        sim.serve("plain")
            .in_cell(cell)
            .mirror::<Position>()
            .mirror::<Actor>()
            .mirror::<Stats>()
            .mirror::<Npc>()
            .add(
                (
                    identity::name_replicated,
                    mirror::deliver,
                    stat::settle_health,
                )
                    .chain(),
            );
    }
    let area = east.id::<AreaDef>("plain");
    let brute = east.id::<NpcDef>("brute");
    let brute = npc::spawn(&mut east.world, brute, Pos::new(270.0, 150.0), area, None);
    combat::inflict(&mut east.world, brute, None, 10.0);
    let hero = east.join(1);
    east.tick();
    let brute_name = *east
        .world
        .get::<GlobalId>(brute)
        .expect("the brute is named");
    let hero_name = *east.world.get::<GlobalId>(hero).expect("the hero is named");

    let deep_in_the_west = Pos::new(220.0, 150.0);
    east.world.get_mut::<Position>(brute).unwrap().pos = deep_in_the_west;
    east.world.get_mut::<Position>(hero).unwrap().pos = deep_in_the_west;
    let moved = hand_over(&worlds, &mut west, &mut east);

    assert_eq!(
        moved,
        vec![Moved {
            client: ClientId(1),
            from: 1,
            to: 0
        }]
    );
    let brute = owned(&west.world, brute_name).expect("the west owns the brute");
    assert_eq!(
        stat::current_health(&west.world, brute),
        15.0,
        "as it stood"
    );
    assert_eq!(position(&west.world, brute), Some(deep_in_the_west));
    assert!(
        owned(&east.world, brute_name).is_none(),
        "the east let it go"
    );
    let hero = owned(&west.world, hero_name).expect("the west owns the hero");
    assert_eq!(
        west.world.resource::<Players>().0.get(&ClientId(1)),
        Some(&hero)
    );
    assert!(
        !east
            .world
            .resource::<Players>()
            .0
            .contains_key(&ClientId(1))
    );

    tick(&worlds, &mut west, &mut east);
    assert!(
        ghost_of(&east.world, brute_name).is_none(),
        "deep in the west, the brute is no ghost in the east"
    );
}

/// Someone in reach of a ghost who wants to deal with it pulls it over: its owner hands it to
/// their world though it stands shallow, so the dealing goes on there.
#[test]
fn a_pull_on_a_ghost_hands_it_over_though_it_stands_shallow() {
    let content = Game::new()
        .area("plain", &Map::open(500, 250), json!([]))
        .hero("plain", 150.0, 150.0)
        .npc("brute", 25.0, json!([]))
        .vfx("damage_taken")
        .shader("damage_pop")
        .load();
    let worlds = Worlds::of(&content);
    let mut west = Sim::new(content.clone());
    let mut east = Sim::new(content);
    for (sim, cell) in [(&mut west, Cell(0)), (&mut east, Cell(1))] {
        sim.serve("plain")
            .in_cell(cell)
            .mirror::<Position>()
            .mirror::<Actor>()
            .mirror::<Stats>()
            .mirror::<Npc>()
            .add((identity::name_replicated, mirror::deliver).chain());
    }
    west.join(1);
    let area = east.id::<AreaDef>("plain");
    let brute = east.id::<NpcDef>("brute");
    let at = Pos::new(262.0, 150.0);
    let brute = npc::spawn(&mut east.world, brute, at, area, None);
    tick(&worlds, &mut west, &mut east);
    let name = *east
        .world
        .get::<GlobalId>(brute)
        .expect("the brute is named");
    let ghost = ghost_of(&west.world, name).expect("a ghost of the brute in the west");

    assert!(mirror::post(
        &mut west.world,
        ghost,
        None,
        mirror::Effect::Pull
    ));
    hand_over(&worlds, &mut west, &mut east);
    hand_over(&worlds, &mut west, &mut east);

    let brute = owned(&west.world, name).expect("the west owns the brute");
    assert_eq!(position(&west.world, brute), Some(at));
    assert!(owned(&east.world, name).is_none(), "the east let it go");
}

/// A player walking over a border walks on without a stop: the world next door takes the
/// character up mid-walk, the connection follows, and the walk ends where it was asked to.
#[test]
fn a_walk_over_a_border_carries_on_into_the_next_cell() {
    let content = Game::new()
        .area("plain", &Map::open(500, 250), json!([]))
        .hero("plain", 230.0, 150.0)
        .load();
    let worlds = Worlds::of(&content);
    let mut cells: Vec<Sim> = (0..2)
        .map(|cell| {
            let mut sim = Sim::new(content.clone());
            sim.serve("plain")
                .in_cell(Cell(cell))
                .mirror::<Position>()
                .mirror::<Actor>()
                .accept::<MoveRequest>()
                .add(
                    (
                        identity::name_replicated,
                        mirror::deliver,
                        movement::move_request,
                        movement::advance,
                    )
                        .chain(),
                );
            sim
        })
        .collect();
    let hero = cells[0].join(1);
    cells[0].world.entity_mut(hero).remove::<ArrivalHold>();
    cells[0].tick();
    let name = *cells[0]
        .world
        .get::<GlobalId>(hero)
        .expect("the hero is named");
    let goal = Pos::new(300.0, 150.0);
    let conn = cells[0].conn(1);
    cells[0].send(
        conn,
        MoveRequest {
            pos: goal,
            seq: MoveSeq(1),
        },
    );
    let clock = *cells[0].world.resource::<WallClock>();
    let mut trail = Vec::new();
    for _ in 0..600 {
        let mut views: Vec<&mut World> = cells.iter_mut().map(|sim| &mut sim.world).collect();
        let moved = worlds::tick(&worlds, &mut views, clock);
        follow(&mut cells, moved);
        let at = cells
            .iter()
            .find_map(|sim| owned(&sim.world, name).and_then(|hero| position(&sim.world, hero)))
            .expect("the hero is owned by one cell");
        trail.push(at);
    }
    assert_eq!(
        trail.last(),
        Some(&goal),
        "walked on to {goal:?}: {trail:?}"
    );
    assert!(
        owned(&cells[1].world, name).is_some(),
        "the east owns the hero at the end"
    );
}

/// A cell nobody plays in or beside stands still, its clock with it, until someone comes: a
/// player beside it wakes it, as their view reaches into it.
#[test]
fn a_cell_nobody_plays_in_or_beside_stands_still_until_someone_comes() {
    let content = Game::new()
        .area("plain", &Map::open(750, 250), json!([]))
        .hero("plain", 100.0, 125.0)
        .load();
    let worlds = Worlds::of(&content);
    assert_eq!(worlds.len(), 3, "a 750 m strip is three cells of 250 m");
    let mut cells: Vec<Sim> = (0..3)
        .map(|cell| {
            let mut sim = Sim::new(content.clone());
            sim.serve("plain")
                .in_cell(Cell(cell))
                .mirror::<Position>()
                .add(identity::name_replicated);
            sim
        })
        .collect();
    let hero = cells[0].join(1);
    let clock = *cells[0].world.resource::<WallClock>();
    let elapsed = |sim: &Sim| sim.world.resource::<Time>().elapsed();

    let before: Vec<_> = cells.iter().map(elapsed).collect();
    let mut views: Vec<&mut World> = cells.iter_mut().map(|sim| &mut sim.world).collect();
    worlds::tick(&worlds, &mut views, clock);
    assert!(elapsed(&cells[0]) > before[0], "the cell played in steps");
    assert!(elapsed(&cells[1]) > before[1], "the cell beside steps");
    assert_eq!(elapsed(&cells[2]), before[2], "the far cell stands still");

    cells[0].world.get_mut::<Position>(hero).unwrap().pos = Pos::new(400.0, 125.0);
    let before: Vec<_> = cells.iter().map(elapsed).collect();
    for _ in 0..2 {
        let mut views: Vec<&mut World> = cells.iter_mut().map(|sim| &mut sim.world).collect();
        let moved = worlds::tick(&worlds, &mut views, clock);
        follow(&mut cells, moved);
    }
    assert!(
        elapsed(&cells[2]) > before[2],
        "the far cell wakes once the player plays beside it"
    );
}

/// Moves each client's connection after its character, as the server does.
fn follow(cells: &mut [Sim], moved: Vec<Moved>) {
    for Moved { client, from, to } in moved {
        let conns: Vec<Entity> = cells[from]
            .world
            .query::<(Entity, &ClientId)>()
            .iter(&cells[from].world)
            .filter(|(_, id)| **id == client)
            .map(|(conn, _)| conn)
            .collect();
        for conn in conns {
            cells[from].world.despawn(conn);
        }
        cells[to].connect(client.0);
    }
}

fn tick(worlds: &Worlds, west: &mut Sim, east: &mut Sim) -> Vec<Moved> {
    let clock = *west.world.resource::<WallClock>();
    worlds::tick(worlds, &mut [&mut west.world, &mut east.world], clock)
}

fn hand_over(worlds: &Worlds, west: &mut Sim, east: &mut Sim) -> Vec<Moved> {
    tick(worlds, west, east)
}

fn owned(world: &World, name: GlobalId) -> Option<Entity> {
    world
        .resource::<Named>()
        .get(name)
        .filter(|&entity| world.get::<Ghost>(entity).is_none())
}

fn ghost_of(world: &World, name: GlobalId) -> Option<Entity> {
    world
        .resource::<Named>()
        .get(name)
        .filter(|&entity| world.get::<Ghost>(entity).is_some())
}
