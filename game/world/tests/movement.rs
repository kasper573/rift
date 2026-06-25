//! `approach` is the shared "walk up to a target" primitive behind both combat and item pickup. Its
//! contract: stop on a tile *beside* the target within range (never on top of it), and issue no move
//! at all when the mover already stands in range.

use bevy_ecs::world::World;
use world::core::tiling::{TilePos, Tiles};
use world::systems::area::{self, AreaTag};
use world::systems::movement::{self, MoveTarget, Position};

const RANGE: Tiles = Tiles(std::f32::consts::SQRT_2);

#[test]
fn approach_stops_within_range_beside_a_far_target() {
    let zone = area::spawn_zone();
    let region = &area::areas()[zone.index()];
    let target = region.spawn;
    let far = *region
        .walkable_nodes
        .iter()
        .max_by(|a, b| a.distance(target).partial_cmp(&b.distance(target)).unwrap())
        .expect("the spawn area has walkable tiles");

    let mut world = World::new();
    let mover = world
        .spawn((Position { pos: far }, AreaTag { area: zone }))
        .id();

    let in_range = movement::approach(&mut world, mover, target, RANGE);

    assert!(!in_range, "a far mover is not yet in range");
    let goal = world
        .get::<MoveTarget>(mover)
        .expect("a far mover heads toward the target")
        .pos;
    assert_ne!(
        goal.cell(),
        target.cell(),
        "it stops beside the target, not on it"
    );
    assert!(goal.distance(target) <= RANGE, "the goal is within range");
}

#[test]
fn approach_issues_no_move_when_already_in_range() {
    let zone = area::spawn_zone();
    let target = area::areas()[zone.index()].spawn;

    let mut world = World::new();
    let mover = world
        .spawn((Position { pos: target }, AreaTag { area: zone }))
        .id();

    let in_range = movement::approach(&mut world, mover, target, RANGE);

    assert!(in_range, "a mover already on the target tile is in range");
    assert!(
        world.get::<MoveTarget>(mover).is_none(),
        "no move is issued when already in range"
    );
}
