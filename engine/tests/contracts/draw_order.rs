use std::cmp::Ordering;

use bevy::prelude::*;
use engine::core::render::{RowDepth, order_rows};

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
