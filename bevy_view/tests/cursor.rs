mod harness;

use bevy_ecs::entity::EntityHashMap;
use bevy_ecs::prelude::*;
use bevy_picking::backend::HitData;
use bevy_picking::hover::HoverMap;
use bevy_picking::pointer::PointerId;
use bevy_view::{CursorIcon, CursorLock, hovered_cursor, node};
use bevy_window::SystemCursorIcon;
use harness::Ui;

const POINTER: CursorIcon = CursorIcon::System(SystemCursorIcon::Pointer);
const RESIZE: CursorIcon = CursorIcon::System(SystemCursorIcon::NwseResize);

fn hover(ui: &mut Ui, entity: Entity) {
    let mut hits = EntityHashMap::default();
    hits.insert(entity, HitData::new(Entity::PLACEHOLDER, 0.0, None, None));
    let mut map = HoverMap::default();
    map.insert(PointerId::Mouse, hits);
    ui.world().insert_resource(map);
}

#[test]
fn hovering_an_element_with_a_cursor_yields_that_cursor() {
    let mut ui = Ui::new();
    ui.render(node().cursor(POINTER));
    let widget = ui.children()[0];
    hover(&mut ui, widget);
    assert_eq!(hovered_cursor(ui.world()), Some(POINTER));
}

#[test]
fn hovering_an_element_without_a_cursor_yields_none() {
    let mut ui = Ui::new();
    ui.render(node());
    let plain = ui.children()[0];
    hover(&mut ui, plain);
    assert_eq!(hovered_cursor(ui.world()), None);
}

#[test]
fn nothing_hovered_yields_none() {
    let mut ui = Ui::new();
    ui.render(node().cursor(POINTER));
    assert_eq!(hovered_cursor(ui.world()), None);
}

#[test]
fn a_lock_overrides_the_hovered_cursor() {
    let mut ui = Ui::new();
    ui.render(node().cursor(POINTER));
    let widget = ui.children()[0];
    hover(&mut ui, widget);
    ui.world().resource_mut::<CursorLock>().0 = Some(RESIZE);
    assert_eq!(
        hovered_cursor(ui.world()),
        Some(RESIZE),
        "a gesture lock wins over hover"
    );
}

#[test]
fn a_lock_holds_even_with_nothing_hovered() {
    let mut ui = Ui::new();
    ui.render(node());
    ui.world().resource_mut::<CursorLock>().0 = Some(RESIZE);
    assert_eq!(
        hovered_cursor(ui.world()),
        Some(RESIZE),
        "the cursor must hold through a gesture even as the pointer leaves the element",
    );
}
