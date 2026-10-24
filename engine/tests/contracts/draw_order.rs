use std::cmp::Ordering;

use bevy::prelude::*;
use engine::core::content::ContentId;
use engine::core::math::Pos;
use engine::core::render::{RowDepth, dynamic_z, order_rows};
use engine::core::units::Meters;
use engine::systems::area::{self, Flip, Scenery};

const ROW: f32 = 3.5;

// A component one sprite gains, as an actor does when its state changes, which moves it to another
// table and so changes the order the renderer meets sprites in.
#[derive(Component)]
struct Reshuffled;

#[test]
fn sprites_sharing_a_row_keep_one_order_as_they_change() {
    let mut app = rows();
    let first = app.world_mut().spawn(on_row(ROW)).id();
    let second = app.world_mut().spawn(on_row(ROW)).id();
    app.update();
    let before = drawn_order(&app, first, second);
    assert_ne!(
        before,
        Ordering::Equal,
        "two sprites on one row drew at one depth"
    );

    let depths = |app: &App| [depth(app, first), depth(app, second)];
    let once = depths(&app);
    app.update();
    assert_eq!(depths(&app), once, "ordering again changes nothing");

    app.world_mut().entity_mut(first).insert(Reshuffled);
    app.update();
    assert_eq!(drawn_order(&app, first, second), before);
}

#[test]
fn a_deeper_row_draws_in_front() {
    let mut app = rows();
    let behind = app.world_mut().spawn(on_row(ROW)).id();
    let in_front = app.world_mut().spawn(on_row(ROW + 0.25)).id();
    app.update();
    assert_eq!(drawn_order(&app, in_front, behind), Ordering::Greater);
}

#[test]
fn scenery_sharing_a_sort_line_draws_in_scene_order_under_who_stands_there() {
    let height = 10.0;
    let base = 4.0;
    let scenery = [
        standing(2.0),
        standing(5.0),
        standing(2.0),
        lying(),
        standing(2.0),
        lying(),
    ];
    let depths = area::scenery_depths(&scenery, height, base);
    let line = dynamic_z(height, base, Meters(4.0));

    assert!(depths[0] < depths[2] && depths[2] < depths[4]);
    assert!(depths[4] < line.next_up());
    assert!(depths[3] < depths[5] && depths[5] < depths[0]);
    assert!(depths[1] > line);
}

fn standing(y: f32) -> Scenery {
    Scenery {
        sort: Some(2.0),
        ..lying_at(y)
    }
}

fn lying() -> Scenery {
    lying_at(1.0)
}

fn lying_at(y: f32) -> Scenery {
    Scenery {
        at: Pos::new(1.0, y),
        tileset: ContentId::new(0),
        tiles: vec![vec![None; 2]; 2],
        sort: None,
        flip: Flip::None,
        blocks: Vec::new(),
    }
}

fn rows() -> App {
    let mut app = App::new();
    app.add_systems(Update, order_rows);
    app
}

fn on_row(row: f32) -> (RowDepth, Transform) {
    (RowDepth(row), Transform::default())
}

fn depth(app: &App, sprite: Entity) -> f32 {
    app.world()
        .get::<Transform>(sprite)
        .expect("a placed sprite")
        .translation
        .z
}

// Which of the two draws on top: greater is in front.
fn drawn_order(app: &App, one: Entity, other: Entity) -> Ordering {
    depth(app, one).total_cmp(&depth(app, other))
}
