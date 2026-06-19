//! `Show` is a conditional subtree: its `when` is read every render, false hides (and unmounts) the
//! body, true reveals (and mounts) it. It nests, and its body may itself be any view including a
//! `For`.

mod harness;

use bevy_ecs::prelude::*;
use bevy_view::{each, node, show, text};
use harness::Ui;

#[derive(Resource)]
struct Flag(bool);

#[test]
fn a_false_condition_renders_nothing() {
    let mut ui = Ui::new();
    ui.render(show(|_| false, node()));
    assert_eq!(ui.child_count(), 0);
}

#[test]
fn a_true_condition_renders_the_body() {
    let mut ui = Ui::new();
    ui.render(show(|_| true, text("shown")));
    assert_eq!(ui.texts(), vec!["shown".to_owned()]);
}

#[test]
fn the_condition_is_re_evaluated_against_the_world_every_render() {
    let mut ui = Ui::new();
    ui.world().insert_resource(Flag(false));
    let view = || show(|w: &World| w.resource::<Flag>().0, text("on"));

    ui.render(view());
    assert_eq!(ui.child_count(), 0);

    ui.world().insert_resource(Flag(true));
    ui.render(view());
    assert_eq!(ui.texts(), vec!["on".to_owned()]);

    ui.world().insert_resource(Flag(false));
    ui.render(view());
    assert_eq!(ui.child_count(), 0);
}

#[test]
fn flipping_false_to_true_reuses_nothing_it_is_a_fresh_mount() {
    let mut ui = Ui::new();
    ui.render(show(|_| true, node()));
    let first = ui.children()[0];
    ui.render(show(|_| false, node()));
    ui.render(show(|_| true, node()));
    let second = ui.children()[0];
    assert_ne!(
        first, second,
        "an unmounted body mounts fresh, not from a ghost entity"
    );
}

#[test]
fn nested_shows_only_reveal_when_both_are_true() {
    let mut ui = Ui::new();
    let view = |outer: bool, inner: bool| show(move |_| outer, show(move |_| inner, text("deep")));

    ui.render(view(true, false));
    assert_eq!(ui.child_count(), 0);
    ui.render(view(false, true));
    assert_eq!(ui.child_count(), 0);
    ui.render(view(true, true));
    assert_eq!(ui.texts(), vec!["deep".to_owned()]);
}

#[test]
fn a_show_whose_body_is_a_for_renders_the_whole_list() {
    let mut ui = Ui::new();
    let view = |on: bool| {
        show(
            move |_| on,
            each(|_| vec![1u64, 2, 3], |&id| id, |&id| text(id.to_string())),
        )
    };
    ui.render(view(true));
    assert_eq!(
        ui.texts(),
        vec!["1".to_owned(), "2".to_owned(), "3".to_owned()]
    );
    ui.render(view(false));
    assert_eq!(ui.child_count(), 0);
}
