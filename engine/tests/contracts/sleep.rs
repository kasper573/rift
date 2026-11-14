use engine::core::chunk::CHUNK_SIDE;
use engine::core::math::{Offset, Pos};
use engine::systems::area::AreaDef;
use engine::systems::movement::{self, MoveTarget, Position, position};
use engine::systems::npc::{self, NpcDef};
use engine::systems::sleep;
use serde_json::json;
use testkit::{Game, Map, Sim};

#[test]
fn what_no_player_is_near_stands_still_until_one_comes() {
    let content = Game::new()
        .area("field", &Map::open(240, 12), json!([]))
        .hero("field", 4.0, 6.0)
        .npc("crow", 5.0, json!([]))
        .load();
    let reach = sleep::awake_reach(&content);
    let mut sim = Sim::new(content);
    sim.serve("field")
        .add(sleep::settle)
        .add(movement::file_by_chunk)
        .plan(movement::AdvanceWalks::default());
    let hero = sim.join(1);
    let (kind, field) = (sim.id::<NpcDef>("crow"), sim.id::<AreaDef>("field"));
    let far = Pos::new(220.0, 6.0);
    let crow = npc::spawn(&mut sim.world, kind, far, field, None);
    sim.world.entity_mut(crow).insert(MoveTarget {
        pos: far - Offset::new(30.0, 0.0),
    });
    let past_reach = Pos::new(4.0 + reach.0 + CHUNK_SIDE.0, 6.0);
    let nearer = npc::spawn(&mut sim.world, kind, past_reach, field, None);
    sim.world.entity_mut(nearer).insert(MoveTarget {
        pos: past_reach + Offset::new(30.0, 0.0),
    });
    sim.run(0.2);
    let resting = position(&sim.world, crow).expect("the crow");
    let nearer_resting = position(&sim.world, nearer).expect("the nearer crow");

    sim.run(2.0);
    assert_eq!(position(&sim.world, crow), Some(resting), "asleep");
    assert_eq!(
        position(&sim.world, nearer),
        Some(nearer_resting),
        "asleep a chunk past what the player reaches, wherever in its chunk the player stands"
    );

    let beside = far - Offset::new(4.0, 0.0);
    sim.world.entity_mut(hero).insert(Position::at(beside));
    sim.run(2.0);
    let moved = position(&sim.world, crow).expect("the crow");
    assert!(
        moved.distance_to(resting) > 1.0,
        "awake once a player is near: {resting:?} to {moved:?}"
    );
}
