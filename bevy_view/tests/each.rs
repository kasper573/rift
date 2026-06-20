mod harness;

use bevy_ecs::prelude::*;
use bevy_view::{each, node};
use harness::Ui;

#[derive(Component)]
struct External(u64);

fn list(ids: &'static [u64]) -> bevy_view::View {
    each(
        move |_| ids.to_vec(),
        |&id| id,
        |&id| node().child(bevy_view::text(id.to_string())),
    )
}

#[test]
fn an_empty_list_renders_no_children() {
    let mut ui = Ui::new();
    ui.render(list(&[]));
    assert_eq!(ui.child_count(), 0);
}

#[test]
fn a_single_item_renders_one_child() {
    let mut ui = Ui::new();
    ui.render(list(&[7]));
    assert_eq!(ui.child_count(), 1);
    assert_eq!(ui.texts(), vec!["7".to_owned()]);
}

#[test]
fn the_child_count_always_matches_the_data() {
    let mut ui = Ui::new();
    ui.render(list(&[1, 2, 3, 4, 5]));
    assert_eq!(ui.child_count(), 5);
    ui.render(list(&[1, 2]));
    assert_eq!(ui.child_count(), 2);
    ui.render(list(&[]));
    assert_eq!(ui.child_count(), 0);
}

#[test]
fn appending_keeps_existing_entities() {
    let mut ui = Ui::new();
    ui.render(list(&[1, 2]));
    let before = ui.children();
    ui.render(list(&[1, 2, 3]));
    let after = ui.children();
    assert_eq!(
        &after[..2],
        &before[..],
        "appended item must not disturb the first two"
    );
    assert_eq!(
        ui.texts(),
        vec!["1".to_owned(), "2".to_owned(), "3".to_owned()]
    );
}

#[test]
fn prepending_keeps_existing_entities_and_reorders() {
    let mut ui = Ui::new();
    ui.render(list(&[1, 2]));
    let before = ui.children();
    ui.render(list(&[0, 1, 2]));
    let after = ui.children();
    assert_eq!(
        &after[1..],
        &before[..],
        "prepended item shifts the originals down, identity intact"
    );
    assert_eq!(
        ui.texts(),
        vec!["0".to_owned(), "1".to_owned(), "2".to_owned()]
    );
}

#[test]
fn inserting_in_the_middle_preserves_the_neighbours() {
    let mut ui = Ui::new();
    ui.render(list(&[1, 3]));
    let before = ui.children();
    ui.render(list(&[1, 2, 3]));
    let after = ui.children();
    assert_eq!(after[0], before[0]);
    assert_eq!(after[2], before[1]);
    assert_eq!(
        ui.texts(),
        vec!["1".to_owned(), "2".to_owned(), "3".to_owned()]
    );
}

#[test]
fn removing_from_the_middle_preserves_the_survivors() {
    let mut ui = Ui::new();
    ui.render(list(&[1, 2, 3]));
    let before = ui.children();
    ui.render(list(&[1, 3]));
    let after = ui.children();
    assert_eq!(after, vec![before[0], before[2]]);
}

#[test]
fn a_full_reverse_keeps_every_entity_and_flips_the_order() {
    let mut ui = Ui::new();
    ui.render(list(&[1, 2, 3, 4]));
    let before = ui.children();
    ui.render(list(&[4, 3, 2, 1]));
    let after = ui.children();
    assert_eq!(after, vec![before[3], before[2], before[1], before[0]]);
    assert_eq!(
        ui.texts(),
        vec![
            "4".to_owned(),
            "3".to_owned(),
            "2".to_owned(),
            "1".to_owned()
        ]
    );
}

#[test]
fn reordering_preserves_identity_and_retained_component_state() {
    let mut ui = Ui::new();
    ui.render(list(&[1, 2, 3]));
    let before = ui.children();
    let entity_for_two = before[1];
    ui.world().entity_mut(entity_for_two).insert(External(42));

    ui.render(list(&[3, 1, 2]));
    let after = ui.children();
    assert_eq!(
        after,
        vec![before[2], before[0], before[1]],
        "stable keys keep their entities"
    );
    assert_eq!(
        ui.world()
            .get::<External>(entity_for_two)
            .map(|state| state.0),
        Some(42),
        "state the view never declared must survive reconciliation",
    );
}

#[test]
fn a_key_that_changes_value_remounts_a_fresh_entity() {
    let mut ui = Ui::new();
    ui.render(list(&[1]));
    let original = ui.children()[0];
    ui.render(list(&[2]));
    let replaced = ui.children()[0];
    assert_ne!(original, replaced, "a different key is a different element");
}

#[test]
fn duplicate_keys_render_every_item_and_keep_the_count() {
    let mut ui = Ui::new();
    ui.render(list(&[1, 1, 1]));
    assert_eq!(ui.child_count(), 3, "duplicate keys must never drop items");
    assert_eq!(
        ui.texts(),
        vec!["1".to_owned(), "1".to_owned(), "1".to_owned()]
    );
}

#[test]
fn duplicate_keys_keep_stable_identity_while_the_duplication_is_stable() {
    let mut ui = Ui::new();
    ui.render(list(&[1, 1]));
    let before = ui.children();
    ui.render(list(&[1, 1]));
    let after = ui.children();
    assert_eq!(
        before, after,
        "a stable duplicate set must reuse its entities"
    );
}
