//! Retention: `attr` writes only the fields the view declares, so component state owned by other
//! systems (a drag system's position, here) survives every render. This is what lets a declarative
//! panel describe its size while a runtime keeps its dragged position.

mod harness;

use bevy_ui::{Node, Val};
use bevy_view::node;
use harness::Ui;

#[test]
fn re_rendering_preserves_fields_the_view_never_declares() {
    let mut ui = Ui::new();
    let view = || {
        node().attr(|entity| {
            let mut style = entity.get_mut::<Node>().unwrap();
            style.width = Val::Px(100.0);
        })
    };

    ui.render(view());
    let panel = ui.children()[0];

    // A runtime system (e.g. dragging) writes a field the view does not own.
    ui.world().entity_mut(panel).get_mut::<Node>().unwrap().left = Val::Px(20.0);

    ui.render(view());
    let style = ui.world().get::<Node>(panel).unwrap();
    assert_eq!(
        style.left,
        Val::Px(20.0),
        "the undeclared position must survive reconciliation"
    );
    assert_eq!(
        style.width,
        Val::Px(100.0),
        "the declared size must still be applied"
    );
}

#[test]
fn the_base_bundle_is_inserted_once_not_every_render() {
    let mut ui = Ui::new();
    ui.render(node());
    let panel = ui.children()[0];
    // If `node`'s base re-ran each render it would reset this to the default; it must not.
    ui.world().entity_mut(panel).get_mut::<Node>().unwrap().top = Val::Px(7.0);
    ui.render(node());
    ui.render(node());
    assert_eq!(ui.world().get::<Node>(panel).unwrap().top, Val::Px(7.0));
}
