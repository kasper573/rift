mod harness;

use bevy_ecs::prelude::*;
use bevy_view::dyn_text;
use harness::Ui;

#[derive(Resource)]
struct Count(u32);

#[test]
fn dynamic_text_re_evaluates_each_render() {
    let mut ui = Ui::new();
    ui.world().insert_resource(Count(0));
    let view = || dyn_text(|w: &World| format!("count {}", w.resource::<Count>().0));

    ui.render(view());
    assert_eq!(ui.texts(), vec!["count 0".to_owned()]);

    ui.world().resource_mut::<Count>().0 = 5;
    ui.render(view());
    assert_eq!(ui.texts(), vec!["count 5".to_owned()]);
}

#[test]
fn the_text_entity_is_stable_across_content_changes() {
    let mut ui = Ui::new();
    ui.world().insert_resource(Count(1));
    let view = || dyn_text(|w: &World| w.resource::<Count>().0.to_string());

    ui.render(view());
    let entity = ui.children()[0];
    ui.world().resource_mut::<Count>().0 = 2;
    ui.render(view());
    assert_eq!(
        ui.children()[0],
        entity,
        "changing text content must not respawn the node"
    );
}

#[test]
fn an_empty_string_is_a_valid_content() {
    let mut ui = Ui::new();
    ui.render(dyn_text(|_| String::new()));
    assert_eq!(ui.texts(), vec![String::new()]);
}
