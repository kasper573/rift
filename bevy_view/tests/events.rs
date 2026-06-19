//! Events: a click runs the element's handler; re-rendering with a new handler replaces it (latest
//! wins, no stacking across renders); a handler may drive state that unmounts its own element on the
//! next render without panicking.

mod harness;

use bevy_ecs::prelude::*;
use bevy_math::Vec2;
use bevy_view::{node, show};
use harness::{Ui, log};

#[test]
fn click_handlers_fan_out_in_order() {
    let mut ui = Ui::new();
    ui.render(node().on_click(|w| log(w, "a")).on_click(|w| log(w, "b")));
    let entity = ui.children()[0];
    ui.activate_click(entity);
    assert_eq!(ui.log(), vec!["a".to_owned(), "b".to_owned()]);
}

#[test]
fn a_drag_dispatches_with_its_delta() {
    let mut ui = Ui::new();
    ui.render(node().on_drag(|w, delta| log(w, format!("drag {} {}", delta.x, delta.y))));
    let entity = ui.children()[0];
    ui.activate_drag(entity, Vec2::new(3.0, 4.0));
    assert_eq!(ui.log(), vec!["drag 3 4".to_owned()]);
}

#[test]
fn over_out_and_drag_end_dispatch() {
    let mut ui = Ui::new();
    ui.render(
        node()
            .on_over(|w| log(w, "over"))
            .on_out(|w| log(w, "out"))
            .on_drag_end(|w| log(w, "end")),
    );
    let entity = ui.children()[0];
    ui.activate_over(entity);
    ui.activate_out(entity);
    ui.activate_drag_end(entity);
    assert_eq!(
        ui.log(),
        vec!["over".to_owned(), "out".to_owned(), "end".to_owned()]
    );
}

#[derive(Resource)]
struct Open(bool);

#[test]
fn a_click_runs_the_handler() {
    let mut ui = Ui::new();
    ui.render(node().on_click(|w| log(w, "clicked")));
    let button = ui.children()[0];
    ui.activate_click(button);
    assert_eq!(ui.log(), vec!["clicked".to_owned()]);
}

#[test]
fn each_activation_runs_the_handler_once() {
    let mut ui = Ui::new();
    ui.render(node().on_click(|w| log(w, "clicked")));
    let button = ui.children()[0];
    ui.activate_click(button);
    ui.activate_click(button);
    assert_eq!(ui.log(), vec!["clicked".to_owned(), "clicked".to_owned()]);
}

#[test]
fn re_rendering_replaces_the_handler_without_stacking() {
    let mut ui = Ui::new();
    ui.render(node().on_click(|w| log(w, "first")));
    let button = ui.children()[0];
    ui.render(node().on_click(|w| log(w, "second")));
    ui.activate_click(button);
    assert_eq!(
        ui.log(),
        vec!["second".to_owned()],
        "only the latest handler runs, exactly once"
    );
}

#[test]
fn a_handler_may_unmount_its_own_element_on_the_next_render() {
    let mut ui = Ui::new();
    ui.world().insert_resource(Open(true));
    let view = || {
        show(
            |w: &World| w.resource::<Open>().0,
            node().on_click(|w| {
                w.resource_mut::<Open>().0 = false;
                log(w, "closed");
            }),
        )
    };
    ui.render(view());
    let button = ui.children()[0];
    ui.activate_click(button);
    ui.render(view());
    assert_eq!(ui.child_count(), 0);
    assert_eq!(ui.log(), vec!["closed".to_owned()]);
}
