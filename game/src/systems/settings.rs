use bevy::prelude::*;
use bevy::scene::EntityScene;
use ui::button::intent as button_intent;
use ui::tokens::typography;
use ui::{Activate, ButtonSize, ValueChange, button_styled};

use crate::systems::hud::{HudAudience, LettersPerSecond, Settings, Window};
use crate::systems::input::map::InputAction;

const SLIDER_WIDTH: f32 = 240.0;

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
        5
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

static SNAPPING: Toggle = Toggle {
    label: |settings| match settings.snapping_enabled() {
        true => "ui snapping enabled".to_owned(),
        false => "ui snapping disabled".to_owned(),
    },
    flip: Settings::toggle_snapping,
};

static REDUCED_MOTION: Toggle = Toggle {
    label: |settings| match settings.reduced_motion() {
        true => "reduced motion on".to_owned(),
        false => "reduced motion off".to_owned(),
    },
    flip: Settings::toggle_reduced_motion,
};

static VOICES: Toggle = Toggle {
    label: |settings| match settings.babble_enabled() {
        true => "voices on".to_owned(),
        false => "voices off".to_owned(),
    },
    flip: Settings::toggle_babble,
};

#[derive(Component, Clone)]
struct ToggleButton(&'static Toggle);

#[derive(Component, Default, Clone)]
struct TextSpeedLabel;

#[derive(Component, Default, Clone)]
struct TextSpeedSlider;

fn content(world: &World) -> Box<dyn Scene> {
    let settings = world.resource::<Settings>();
    let speed = settings.letters_per_second();
    Box::new(bsn! {
        Node { flex_direction: FlexDirection::Column, align_items: AlignItems::Start, row_gap: Val::Px(6.0) }
        Children [
            {EntityScene(toggle_button(settings, &SNAPPING))},
            (
                Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(6.0), width: Val::Px(SLIDER_WIDTH), padding: {UiRect::vertical(Val::Px(6.0))} }
                Children [
                    ( {ui::styled_text(speed_label(speed), ui::theme::theme().surface_canvas.on, typography::BODY)} TextSpeedLabel ),
                    (
                        {ui::slider(speed.0, LettersPerSecond::SLOWEST.0, LettersPerSecond::FASTEST.0)}
                        TextSpeedSlider
                        on(|changed: On<ValueChange<f32>>, mut settings: ResMut<Settings>| {
                            settings.set_letters_per_second(LettersPerSecond(changed.value.round()));
                        })
                        Children [
                            ( {ui::slider_track()}
                              Children [ {EntityScene(ui::slider_range())}, {EntityScene(ui::slider_thumb())} ]
                            )
                        ]
                    ),
                ]
            ),
            {EntityScene(toggle_button(settings, &REDUCED_MOTION))},
            {EntityScene(toggle_button(settings, &VOICES))},
        ]
    })
}

fn toggle_button(settings: &Settings, toggle: &'static Toggle) -> impl Scene + use<> {
    bsn! {
        {button_styled(button_intent::PRIMARY, ButtonSize::Md, (toggle.label)(settings))}
        ui::component(ToggleButton(toggle))
        on(move |_: On<Activate>, mut commands: Commands| {
            commands.queue(move |world: &mut World| {
                (toggle.flip)(&mut world.resource_mut::<Settings>());
            });
        })
    }
}

fn speed_label(speed: LettersPerSecond) -> String {
    format!("text speed {} letters a second", speed.0 as u32)
}

fn sync_labels(world: &mut World) {
    if !world.is_resource_changed::<Settings>() {
        return;
    }
    let buttons: Vec<(&'static Toggle, Vec<Entity>)> = world
        .query::<(&ToggleButton, &Children)>()
        .iter(world)
        .map(|(button, children)| (button.0, children.iter().collect()))
        .collect();
    let speed = world.resource::<Settings>().letters_per_second();
    let labels: Vec<(Entity, String)> = world
        .query_filtered::<Entity, With<TextSpeedLabel>>()
        .iter(world)
        .map(|label| (label, speed_label(speed)))
        .collect();
    let texts: Vec<(Entity, String)> = buttons
        .into_iter()
        .flat_map(|(toggle, texts)| {
            let label = (toggle.label)(world.resource::<Settings>());
            texts.into_iter().map(move |text| (text, label.clone()))
        })
        .chain(labels)
        .collect();
    for (entity, label) in texts {
        if let Some(mut text) = world.get_mut::<Text>(entity)
            && text.0 != label
        {
            text.0 = label;
        }
    }
}
