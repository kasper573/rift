//! Composition: deep nesting, fragments and empties returned by component-style functions, a slot
//! whose element type changes (a swap, not an in-place mutation), a conditional subtree inside a
//! list item, and — the load-bearing one — static sibling slots that keep their identity even as a
//! neighbouring `For` grows and shrinks between them.

mod harness;

use bevy_ecs::prelude::Component;
use bevy_view::{Element, View, boundary, each, node, show, text};
use harness::Ui;

#[derive(Component, Clone)]
struct Plain;
#[derive(Component, Clone)]
struct Wrapped;

/// A component: an ordinary function returning a `View`, composed like any element.
fn labelled(value: &str) -> View {
    node().child(text(value.to_owned())).into()
}

#[test]
fn components_compose_like_elements() {
    let mut ui = Ui::new();
    ui.render(node().child(labelled("a")).child(labelled("b")));
    assert_eq!(ui.texts(), vec!["a".to_owned(), "b".to_owned()]);
}

#[test]
fn a_component_returning_empty_contributes_no_entity() {
    let mut ui = Ui::new();
    ui.render(
        node()
            .child(text("x"))
            .child(View::empty())
            .child(text("y")),
    );
    assert_eq!(ui.texts(), vec!["x".to_owned(), "y".to_owned()]);
}

#[test]
fn a_component_returning_a_fragment_expands_in_place() {
    let mut ui = Ui::new();
    let pair = || View::fragment([text("one").into(), text("two").into()]);
    ui.render(
        node()
            .child(text("before"))
            .child(pair())
            .child(text("after")),
    );
    assert_eq!(
        ui.texts(),
        vec![
            "before".to_owned(),
            "one".to_owned(),
            "two".to_owned(),
            "after".to_owned()
        ],
    );
}

#[test]
fn deep_nesting_renders_every_level() {
    let mut ui = Ui::new();
    ui.render(node().child(node().child(node().child(text("bottom")))));
    assert_eq!(ui.texts(), vec!["bottom".to_owned()]);
}

#[test]
fn swapping_the_element_type_at_a_slot_replaces_the_entity() {
    let mut ui = Ui::new();
    let view = |as_text: bool| -> View {
        if as_text {
            text("now text").into()
        } else {
            node().into()
        }
    };
    ui.render(view(false));
    let as_node = ui.children()[0];
    ui.render(view(true));
    let as_text = ui.children()[0];
    assert_ne!(
        as_node, as_text,
        "a type change must despawn and respawn, not mutate in place"
    );
    assert_eq!(ui.texts(), vec!["now text".to_owned()]);
}

#[test]
fn swapping_a_plain_node_for_a_boundary_wrapped_one_remounts() {
    let mut ui = Ui::new();
    // The game's window/widget case: a slot that is a plain `node` (a window frame) or a
    // `boundary`-wrapped `node` (a component). They share a path and tag but differ in instance, so the
    // slot must remount — otherwise the plain node's frame survives onto the component and never goes.
    let view = |wrapped: bool| -> View {
        if wrapped {
            boundary(node().insert(Wrapped))
        } else {
            node().insert(Plain).into()
        }
    };

    ui.render(view(false));
    let plain = ui.children()[0];
    assert!(ui.has::<Plain>(plain));

    ui.render(view(true));
    let wrapped = ui.children()[0];
    assert_ne!(
        plain, wrapped,
        "a plain->boundary swap must remount, not reuse"
    );
    assert!(ui.has::<Wrapped>(wrapped));
    assert!(
        !ui.has::<Plain>(wrapped),
        "the plain node's components must not graft onto the boundary-wrapped entity"
    );
}

#[test]
fn static_sibling_slots_keep_identity_as_a_neighbouring_for_resizes() {
    let mut ui = Ui::new();
    let panel = |ids: &'static [u64]| -> Element {
        node().children([
            text("head").into(),
            each(move |_| ids.to_vec(), |&id| id, |&id| text(id.to_string())),
            text("tail").into(),
        ])
    };

    ui.render(panel(&[1, 2]));
    let container = ui.children()[0];
    let before = ui.children_of(container);
    let head = before[0];
    let tail = *before.last().unwrap();

    ui.render(panel(&[1, 2, 3, 4]));
    let after = ui.children_of(container);
    assert_eq!(
        after[0], head,
        "the head slot is fixed regardless of list size"
    );
    assert_eq!(
        *after.last().unwrap(),
        tail,
        "the tail slot survives the list growing past it"
    );
    assert_eq!(
        ui.texts(),
        vec![
            "head".to_owned(),
            "1".to_owned(),
            "2".to_owned(),
            "3".to_owned(),
            "4".to_owned(),
            "tail".to_owned(),
        ],
    );
}

#[test]
fn a_conditional_subtree_inside_a_list_item_toggles_per_item() {
    let mut ui = Ui::new();
    // Each item shows its id, and — only for even ids — an extra "even" marker line.
    let view = || {
        each(
            |_| vec![1u64, 2, 3, 4],
            |&id| id,
            |&id| {
                node()
                    .child(text(id.to_string()))
                    .child(show(move |_| id % 2 == 0, text("even")))
            },
        )
    };
    ui.render(view());
    assert_eq!(
        ui.texts(),
        vec![
            "1".to_owned(),
            "2".to_owned(),
            "even".to_owned(),
            "3".to_owned(),
            "4".to_owned(),
            "even".to_owned(),
        ],
    );
}
