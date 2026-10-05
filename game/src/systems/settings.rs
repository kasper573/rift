use bevy::prelude::*;
use ui::button::intent as button_intent;
use ui::{Activate, ButtonSize, button_styled};

use crate::systems::hud::{HudAudience, Settings, Window};
use crate::systems::input::map::InputAction;

pub struct SettingsWindow;

impl Window for SettingsWindow {
    fn audience(&self) -> HudAudience {
        HudAudience::Everyone
    }
    fn title(&self) -> &'static str {
        "Settings"
    }
    fn toggle(&self) -> InputAction {
        InputAction::ToggleSettings
    }
    fn icon(&self) -> &'static str {
        "icons/misc/gear.png"
    }
    fn order(&self) -> u32 {
        4
    }
    fn contents(&self, world: &World) -> Vec<ui::WindowContent> {
        crate::systems::hud::single_tab(self.title(), ui::scrolled(content(world)))
    }
    fn sync(&self, world: &mut World) {
        sync_labels(world)
    }
}

struct Toggle {
    label: fn(&Settings) -> String,
    flip: fn(&mut Settings),
}

static TOGGLES: &[Toggle] = &[
    Toggle {
        label: |settings| match settings.snapping_enabled() {
            true => "ui snapping enabled".to_owned(),
            false => "ui snapping disabled".to_owned(),
        },
        flip: Settings::toggle_snapping,
    },
    Toggle {
        label: |settings| format!("text speed {}", settings.text_speed().label()),
        flip: Settings::cycle_text_speed,
    },
    Toggle {
        label: |settings| match settings.reduced_motion() {
            true => "reduced motion on".to_owned(),
            false => "reduced motion off".to_owned(),
        },
        flip: Settings::toggle_reduced_motion,
    },
];

#[derive(Component, Default, Clone)]
struct ToggleButton {
    index: usize,
}

fn content(world: &World) -> Box<dyn Scene> {
    let settings = world.resource::<Settings>();
    let buttons: Vec<Box<dyn Scene>> = TOGGLES
        .iter()
        .enumerate()
        .map(|(index, toggle)| -> Box<dyn Scene> {
            Box::new(bsn! {
                {button_styled(button_intent::PRIMARY, ButtonSize::Md, (toggle.label)(settings))}
                ToggleButton { index: {index} }
                on(move |_: On<Activate>, mut commands: Commands| {
                    commands.queue(move |world: &mut World| {
                        (TOGGLES[index].flip)(&mut world.resource_mut::<Settings>());
                    });
                })
            })
        })
        .collect();
    Box::new(bsn! {
        Node { flex_direction: FlexDirection::Column, align_items: AlignItems::Start, row_gap: Val::Px(6.0) }
        Children [ {buttons} ]
    })
}

fn sync_labels(world: &mut World) {
    if !world.is_resource_changed::<Settings>() {
        return;
    }
    let buttons: Vec<(usize, Vec<Entity>)> = world
        .query::<(&ToggleButton, &Children)>()
        .iter(world)
        .map(|(button, children)| (button.index, children.iter().collect()))
        .collect();
    for (index, texts) in buttons {
        let label = (TOGGLES[index].label)(world.resource::<Settings>());
        for entity in texts {
            if let Some(mut text) = world.get_mut::<Text>(entity)
                && text.0 != label
            {
                text.0 = label.clone();
            }
        }
    }
}
