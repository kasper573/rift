//! Foundations: an empty view renders nothing, primitives mount under the host, and re-rendering an
//! unchanged tree leaves the entities untouched.

mod harness;

use bevy_view::{View, node, text};
use harness::Ui;

#[test]
fn an_empty_view_renders_no_children() {
    let mut ui = Ui::new();
    ui.render(View::empty());
    assert_eq!(ui.child_count(), 0);
}

#[test]
fn a_node_mounts_a_single_child() {
    let mut ui = Ui::new();
    ui.render(node());
    assert_eq!(ui.child_count(), 1);
}

#[test]
fn static_text_renders_its_content() {
    let mut ui = Ui::new();
    ui.render(text("hello"));
    assert_eq!(ui.texts(), vec!["hello".to_owned()]);
}

#[test]
fn a_fragment_mounts_each_member_in_order() {
    let mut ui = Ui::new();
    ui.render(View::fragment([
        text("a").into(),
        text("b").into(),
        text("c").into(),
    ]));
    assert_eq!(ui.child_count(), 3);
    assert_eq!(
        ui.texts(),
        vec!["a".to_owned(), "b".to_owned(), "c".to_owned()]
    );
}

#[test]
fn re_rendering_an_unchanged_tree_keeps_the_same_entities() {
    let mut ui = Ui::new();
    ui.render(node().child(text("x")));
    let before = ui.children();
    ui.render(node().child(text("x")));
    let after = ui.children();
    assert_eq!(
        before, after,
        "stable structure must reuse entities, not respawn them"
    );
    assert_eq!(ui.texts(), vec!["x".to_owned()]);
}
