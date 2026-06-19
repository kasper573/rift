//! Intrinsic tags are real `bevy_ui` primitives: `<button>` inserts `Button`, `<image>` inserts
//! `ImageNode`, and swapping an element's tag at a slot is a type change — despawn + respawn, not a
//! mutate in place.

mod harness;

use bevy_ui::prelude::{Button, ImageNode, Node};
use bevy_view::{button, image, node, view};
use harness::Ui;

#[test]
fn button_inserts_the_button_primitive() {
    let mut ui = Ui::new();
    ui.render(button());
    let entity = ui.children()[0];
    assert!(ui.has::<Button>(entity));
    assert!(ui.has::<Node>(entity));
}

#[test]
fn image_inserts_the_image_primitive() {
    let mut ui = Ui::new();
    ui.render(image(ImageNode::default()));
    let entity = ui.children()[0];
    assert!(ui.has::<ImageNode>(entity));
    assert!(ui.has::<Node>(entity));
}

#[test]
fn macro_button_and_image_map_to_primitives() {
    let mut ui = Ui::new();
    ui.render(view! { <button><image src={ImageNode::default()}/></button> });
    let btn = ui.children()[0];
    assert!(ui.has::<Button>(btn));
    let img = ui.children_of(btn)[0];
    assert!(ui.has::<ImageNode>(img));
}

#[test]
fn re_rendering_an_image_follows_a_changed_source() {
    let mut ui = Ui::new();
    let flipped = |flip: bool| {
        image(ImageNode {
            flip_x: flip,
            ..Default::default()
        })
    };

    ui.render(flipped(false));
    let entity = ui.children()[0];

    ui.render(flipped(true));
    assert_eq!(
        ui.children()[0],
        entity,
        "the image entity is reused across renders"
    );
    assert!(
        ui.world().get::<ImageNode>(entity).unwrap().flip_x,
        "a re-rendered image must follow its new source, not keep the one it mounted with"
    );
}

#[test]
fn swapping_intrinsic_tag_respawns_the_entity() {
    let mut ui = Ui::new();
    ui.render(node());
    let before = ui.children()[0];
    ui.render(button());
    let after = ui.children()[0];
    assert_ne!(
        before, after,
        "a node->button swap is a type change: despawn + respawn"
    );
    assert!(ui.has::<Button>(after));
}
