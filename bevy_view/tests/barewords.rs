mod harness;

use bevy_ui::{Node, PositionType, Val};
use bevy_view::view;
use harness::Ui;

#[test]
fn bareword_attrs_set_node_fields() {
    let mut ui = Ui::new();
    ui.render(view! { <node width=Val::Px(120.0) position_type=PositionType::Absolute/> });
    let node = ui.get::<Node>(ui.children()[0]).unwrap();
    assert_eq!(node.width, Val::Px(120.0));
    assert_eq!(node.position_type, PositionType::Absolute);
}

#[test]
fn the_setter_is_partial_so_runtime_owned_fields_survive() {
    let mut ui = Ui::new();
    let view = || view! { <node width=Val::Px(50.0)/> };
    ui.render(view());
    let entity = ui.children()[0];
    {
        let mut node = ui.world().get_mut::<Node>(entity).unwrap();
        node.left = Val::Px(17.0);
        node.top = Val::Px(33.0);
    }

    ui.render(view());
    let node = ui.get::<Node>(entity).unwrap();
    assert_eq!(node.width, Val::Px(50.0));
    assert_eq!(
        node.left,
        Val::Px(17.0),
        "the view never declared left; the reconciler must not clobber it"
    );
    assert_eq!(node.top, Val::Px(33.0));
}
