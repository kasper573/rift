use bevy::prelude::*;
use ui::button::intent as button_intent;
use ui::{Activate, ButtonSize, button_styled};

use super::{Settings, Window};

#[derive(Component, Default, Clone)]
struct SnappingButton;

pub struct SettingsWindow;

impl Window for SettingsWindow {
    fn title(&self) -> &'static str {
        "Settings"
    }
    fn toggle(&self) -> KeyCode {
        KeyCode::KeyO
    }
    fn keybind(&self) -> &'static str {
        "O"
    }
    fn icon(&self) -> &'static str {
        "icons/misc/gear.png"
    }
    fn order(&self) -> u32 {
        3
    }
    fn contents(&self, _: &World) -> Vec<ui::WindowContent> {
        super::single_tab(self.title(), ui::scrolled(content()))
    }
    fn sync(&self, world: &mut World) {
        sync_snapping(world)
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
