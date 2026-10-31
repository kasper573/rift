use bevy_ecs::prelude::*;
use engine::core::math::Pos;
use engine::systems::actor::Actor;
use engine::systems::area::{AreaDef, Cell};
use engine::systems::combat;
use engine::systems::identity::{self, GlobalId, Named};
use engine::systems::mirror::{self, Ghost};
use engine::systems::movement::{Position, position};
use engine::systems::npc::{self, Npc, NpcDef};
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
        .area("plain", &Map::open(300, 300), json!([]))
        .hero("plain", 150.0, 150.0)
        .npc("brute", 25.0, json!([]))
        .vfx("damage_taken")
        .shader("damage_pop")
        .load();
    let worlds = Worlds::of(&content);
    assert_eq!(worlds.len(), 4, "a 300 m area is four cells");

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

fn tick(worlds: &Worlds, west: &mut Sim, east: &mut Sim) {
    west.tick();
    east.tick();
    mirror::exchange(worlds, &mut [&mut west.world, &mut east.world]);
}

fn ghost_of(world: &World, name: GlobalId) -> Option<Entity> {
    world
        .resource::<Named>()
        .get(name)
        .filter(|&entity| world.get::<Ghost>(entity).is_some())
}
