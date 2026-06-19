//! The `draggable`/`resizable` behaviors: a handle moves/resizes the nearest movable root; `initial`
//! seeds geometry exactly once (a live drag survives re-renders); a tap is suppressed when it is the
//! tail of a drag; `on_settle` is given the final geometry and decides where the panel settles.

mod harness;

use bevy_math::Vec2;
use bevy_ui::{Node, Val};
use bevy_view::{Geom, View, draggable, node, resizable, view};
use harness::{Ui, log};

fn round_to_ten(value: f32) -> f32 {
    (value / 10.0).round() * 10.0
}

#[test]
fn initial_seeds_geometry_once_and_a_drag_survives_re_render() {
    let mut ui = Ui::new();
    let view = || node().bind(draggable().initial(Vec2::new(10.0, 20.0)).whole());
    ui.render(view());
    let entity = ui.children()[0];
    assert_eq!(ui.get::<Node>(entity).unwrap().left, Val::Px(10.0));
    assert_eq!(ui.get::<Node>(entity).unwrap().top, Val::Px(20.0));

    ui.activate_drag(entity, Vec2::new(5.0, -4.0));
    assert_eq!(ui.get::<Node>(entity).unwrap().left, Val::Px(15.0));
    assert_eq!(ui.get::<Node>(entity).unwrap().top, Val::Px(16.0));

    ui.render(view());
    assert_eq!(
        ui.get::<Node>(entity).unwrap().left,
        Val::Px(15.0),
        "initial seeds once; a re-render must not reset the dragged position"
    );
}

#[test]
fn a_handle_moves_the_nearest_root_not_itself() {
    fn window() -> View {
        let drag = draggable().initial(Vec2::ZERO);
        node()
            .bind(drag.root())
            .child(node().bind(drag.handle()))
            .into()
    }
    let mut ui = Ui::new();
    ui.render(window());
    let root = ui.children()[0];
    let handle = ui.children_of(root)[0];

    ui.activate_drag(handle, Vec2::new(7.0, 9.0));
    assert_eq!(ui.get::<Node>(root).unwrap().left, Val::Px(7.0));
    assert_eq!(ui.get::<Node>(root).unwrap().top, Val::Px(9.0));
    assert_eq!(
        ui.get::<Node>(handle).unwrap().left,
        Val::Auto,
        "dragging the handle moves the root, never the handle itself"
    );
}

#[test]
fn a_tap_is_suppressed_after_a_drag_but_fires_on_a_clean_click() {
    let mut ui = Ui::new();
    let view = || {
        node().bind(
            draggable()
                .initial(Vec2::ZERO)
                .on_tap(|w| log(w, "tap"))
                .whole(),
        )
    };
    ui.render(view());
    let entity = ui.children()[0];

    ui.activate_click(entity);
    assert_eq!(ui.log(), vec!["tap".to_owned()], "a clean click taps");
    ui.clear_log();

    ui.activate_drag(entity, Vec2::new(3.0, 0.0));
    ui.activate_click(entity);
    assert_eq!(
        ui.log(),
        Vec::<String>::new(),
        "the click that ends a drag does not tap"
    );

    ui.activate_click(entity);
    assert_eq!(
        ui.log(),
        vec!["tap".to_owned()],
        "the next clean click taps again"
    );
}

#[test]
fn on_settle_receives_the_geometry_and_decides_where_it_lands() {
    let mut ui = Ui::new();
    let view = || {
        node().bind(
            draggable()
                .initial(Vec2::new(2.0, 3.0))
                .on_settle(|world, geom| {
                    log(world, format!("settle {} {}", geom.pos.x, geom.pos.y));
                    Geom {
                        pos: Vec2::new(round_to_ten(geom.pos.x), round_to_ten(geom.pos.y)),
                        size: geom.size,
                    }
                })
                .whole(),
        )
    };
    ui.render(view());
    let entity = ui.children()[0];

    ui.activate_drag(entity, Vec2::new(14.0, 0.0)); // pos -> (16, 3)
    ui.activate_drag_end(entity);

    assert_eq!(ui.log(), vec!["settle 16 3".to_owned()]);
    assert_eq!(
        ui.get::<Node>(entity).unwrap().left,
        Val::Px(20.0),
        "the node settles to the snapped geometry the callback returned"
    );
    assert_eq!(ui.get::<Node>(entity).unwrap().top, Val::Px(0.0));
}

#[test]
fn resize_grows_and_shrinks_clamped_to_a_minimum() {
    fn window() -> View {
        let drag = draggable()
            .initial(Vec2::ZERO)
            .initial_size(Vec2::new(100.0, 100.0));
        let resize = resizable().min(Vec2::new(50.0, 50.0));
        view! { <node use={drag.root()} use={resize.handle()}/> }
    }
    let mut ui = Ui::new();
    ui.render(window());
    let entity = ui.children()[0];
    assert_eq!(ui.get::<Node>(entity).unwrap().width, Val::Px(100.0));

    ui.activate_drag(entity, Vec2::new(-80.0, -80.0));
    assert_eq!(
        ui.get::<Node>(entity).unwrap().width,
        Val::Px(50.0),
        "shrinking is clamped to the minimum"
    );
    assert_eq!(ui.get::<Node>(entity).unwrap().height, Val::Px(50.0));

    ui.activate_drag(entity, Vec2::new(30.0, 10.0));
    assert_eq!(ui.get::<Node>(entity).unwrap().width, Val::Px(80.0));
    assert_eq!(ui.get::<Node>(entity).unwrap().height, Val::Px(60.0));
}
