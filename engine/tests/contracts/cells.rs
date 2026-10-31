use bevy_ecs::prelude::*;
use engine::core::math::Pos;
use engine::systems::actor::Actor;
use engine::systems::area::{AreaDef, Cell};
use engine::systems::combat;
use engine::systems::handoff::{self, Moved};
use engine::systems::identity::{self, GlobalId, Named};
use engine::systems::mirror::{self, Ghost};
use engine::systems::movement::{Position, position};
use engine::systems::npc::{self, Npc, NpcDef};
use engine::systems::player::{ClientId, Players};
use engine::systems::stat::{self, Stats};
use engine::systems::worlds::Worlds;
use serde_json::json;
use testkit::{Game, Map, Sim};

/// An area wider than a cell runs as one world per cell. What one world owns near a border, the
/// world next door holds as a ghost standing where its owner does, gone once it leaves the halo;
/// a blow dealt to the ghost lands on its owner a tick later.
#[test]
fn an_actor_near_a_border_is_a_ghost_next_door_and_a_blow_on_the_ghost_lands_on_it() {
    let content = Game::new()
        .area("plain", &Map::open(500, 500), json!([]))
        .hero("plain", 150.0, 150.0)
        .npc("brute", 25.0, json!([]))
        .vfx("damage_taken")
        .shader("damage_pop")
        .load();
    let worlds = Worlds::of(&content);
    assert_eq!(worlds.len(), 4, "a 500 m area is four cells of 250 m");

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
        .area("plain", &Map::open(500, 500), json!([]))
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
        .area("plain", &Map::open(500, 500), json!([]))
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

fn tick(worlds: &Worlds, west: &mut Sim, east: &mut Sim) {
    west.tick();
    east.tick();
    mirror::exchange(worlds, &mut [&mut west.world, &mut east.world]);
}

fn hand_over(worlds: &Worlds, west: &mut Sim, east: &mut Sim) -> Vec<Moved> {
    tick(worlds, west, east);
    handoff::handoff(worlds, &mut [&mut west.world, &mut east.world])
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
