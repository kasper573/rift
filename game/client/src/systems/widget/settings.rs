//! The settings window: a toggle for UI snapping.

use bevy::prelude::*;
use ui::button::intent as button_intent;
use ui::{Activate, ButtonSize, button_styled};

use super::{Settings, WindowDef};

#[derive(Component, Default, Clone)]
struct SnappingButton;

inventory::submit! {
    WindowDef {
        id: "Settings",
        title: "Settings",
        toggle: KeyCode::KeyO,
        keybind: "O",
        icon: "icons/misc/gear.png",
        order: 3,
        content,
        sync: sync_snapping,
    }
}

fn content() -> Box<dyn Scene> {
    Box::new(bsn! {
        {button_styled(button_intent::PRIMARY, ButtonSize::Md, "ui snapping disabled")}
        SnappingButton
        on(|_: On<Activate>, mut commands: Commands| {
            commands.queue(toggle_snapping);
        })
    })
}

fn sync_snapping(world: &mut World) {
    let label = if world.resource::<Settings>().snapping_enabled() {
        "ui snapping enabled"
    } else {
        "ui snapping disabled"
    };
    let texts: Vec<Entity> = world
        .query_filtered::<&Children, With<SnappingButton>>()
        .iter(world)
        .flat_map(|children| children.iter())
        .collect();
    for entity in texts {
        if let Some(mut text) = world.get_mut::<Text>(entity) {
            text.0 = label.to_owned();
        }
    }
}

fn toggle_snapping(world: &mut World) {
    world.resource_mut::<Settings>().toggle_snapping();
}
