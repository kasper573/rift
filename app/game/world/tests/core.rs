use actor::{ActorModel, Direction};
use world::core::actors::{model_index, models};
use world::core::area::{areas, spawn_zone};
use world::core::math::{Pos, Tiles};
use world::core::protocol::action_name;
use world::{ACTION_ATTACK, ACTION_DEAD, ACTION_IDLE, ACTION_RUN, ACTION_WALK};

#[test]
fn vector_ops() {
    let p = |x, y| Pos::new(Tiles(x), Tiles(y));
    assert_eq!(p(1.0, 2.0) + p(3.0, 4.0), p(4.0, 6.0));
    assert!((p(3.0, 4.0).length() - 5.0).abs() < 1e-6);
    assert!((p(0.0, 0.0).distance(p(0.0, 5.0)) - 5.0).abs() < 1e-6);
}

#[test]
fn dir8_cardinals() {
    assert_eq!(Direction::from_vec(1.0, 0.0), Direction::E);
    assert_eq!(Direction::from_vec(0.0, 1.0), Direction::S);
    assert_eq!(Direction::from_vec(0.0, -1.0), Direction::N);
    assert_eq!(Direction::from_vec(-1.0, 0.0), Direction::W);
}

fn adventurer() -> &'static ActorModel {
    let index = model_index("adventurer").expect("the adventurer model");
    &models()[index.0 as usize]
}

#[test]
fn actor_walk_slice_faces_movement() {
    let walk = action_name(ACTION_WALK);
    let slice = |dir: Direction| adventurer().frame(walk, dir as u8, 0.0, 1.0);

    let declared = [
        Direction::S,
        Direction::SW,
        Direction::NW,
        Direction::N,
        Direction::NE,
        Direction::SE,
    ];
    for (index, a) in declared.iter().enumerate() {
        for b in &declared[index + 1..] {
            assert_ne!(
                slice(*a),
                slice(*b),
                "{a:?} and {b:?} need their own slices"
            );
        }
    }

    assert_eq!(
        slice(Direction::E),
        slice(Direction::SE),
        "east should fall back to se"
    );
    assert_eq!(
        slice(Direction::W),
        slice(Direction::SW),
        "west should fall back to sw — se would face right while moving left (moonwalk)",
    );
}

#[test]
fn every_model_animates_every_action_and_direction() {
    assert!(!models().is_empty(), "the assets must define actor models");
    for model in models() {
        let image = model.image();
        assert!(image.width > 0 && image.height > 0);
        for action in [
            ACTION_IDLE,
            ACTION_WALK,
            ACTION_RUN,
            ACTION_ATTACK,
            ACTION_DEAD,
        ] {
            for dir in 0..8 {
                let frame = model.frame(action_name(action), dir, 0.5, 1.0);
                assert!(frame.size.x.0 > 0.0 && frame.size.y.0 > 0.0);
                assert!(frame.pos.x.0 + frame.size.x.0 <= image.width as f32);
                assert!(frame.pos.y.0 + frame.size.y.0 <= image.height as f32);
            }
        }
    }
}

#[test]
fn death_plays_once_then_holds() {
    let death = action_name(ACTION_DEAD);
    assert_ne!(
        adventurer().frame(death, 0, 0.0, 1.0),
        adventurer().frame(death, 0, 5.0, 1.0)
    );
    assert_eq!(
        adventurer().frame(death, 0, 5.0, 1.0),
        adventurer().frame(death, 0, 9.0, 1.0)
    );
}

#[test]
fn walking_cycles_over_time() {
    let walk = action_name(ACTION_WALK);
    assert_ne!(
        adventurer().frame(walk, 0, 0.0, 1.0),
        adventurer().frame(walk, 0, 0.25, 1.0)
    );
}

#[test]
fn every_area_is_walkable_pathable_and_connected() {
    assert!(
        !areas().is_empty(),
        "the assets must define at least one map"
    );
    for area in areas() {
        let name = &area.name;
        assert!(
            area.width.0 > 0.0 && area.height.0 > 0.0,
            "{name} must have tiles"
        );

        let spawn = area.spawn;
        assert!(
            area.grid.walkable(spawn),
            "{name}: spawn tile must be walkable"
        );
        assert!(!area.portals.is_empty(), "{name} must have a portal");
        for portal in &area.portals {
            let dest = areas()
                .get(portal.dest_area.0 as usize)
                .unwrap_or_else(|| panic!("{name}: portal must lead to an existing area"));
            assert!(
                dest.grid.walkable(portal.dest),
                "{name}: portal exit in {} must be walkable",
                dest.name,
            );
        }
        assert!(
            area.portals.iter().any(|portal| {
                let tile = portal.rect.center();
                area.grid.walkable(tile) && nav::astar(&area.grid, spawn, tile).is_some()
            }),
            "{name}: a portal must be reachable from spawn",
        );
    }
}

#[test]
fn one_area_is_the_spawn_zone() {
    assert!((spawn_zone().0 as usize) < areas().len());
}

// The same call the server's build script makes: every loader runs, every
// cross-reference resolves, or this panics naming the offending file and row.
#[test]
fn embedded_content_validates() {
    world::validate();
}
