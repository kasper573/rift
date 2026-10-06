use bevy::prelude::*;
use bevy::scene::EntityScene;
use ui::button::intent as button_intent;
use ui::tokens::typography;
use ui::{Activate, ButtonSize, ValueChange, button_styled};

use crate::core::sfx::{AudioCategory, AudioFader, AudioVolume};
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

#[derive(Clone, Copy)]
enum SettingSlider {
    TextSpeed,
    Volume(AudioFader),
}

const VOLUMES: [SettingSlider; 4] = [
    SettingSlider::Volume(AudioFader::Master),
    SettingSlider::Volume(AudioFader::Category(AudioCategory::Music)),
    SettingSlider::Volume(AudioFader::Category(AudioCategory::Voice)),
    SettingSlider::Volume(AudioFader::Category(AudioCategory::Effects)),
];

#[derive(Component, Clone)]
struct ToggleButton(&'static Toggle);

#[derive(Component, Clone)]
struct SliderLabel(SettingSlider);

fn content(world: &World) -> Box<dyn Scene> {
    let settings = world.resource::<Settings>();
    let volumes: Vec<Box<dyn Scene>> = VOLUMES
        .into_iter()
        .map(|slider| -> Box<dyn Scene> { Box::new(slider_row(settings, slider)) })
        .collect();
    Box::new(bsn! {
        Node { flex_direction: FlexDirection::Column, align_items: AlignItems::Start, row_gap: Val::Px(6.0) }
        Children [
            {EntityScene(toggle_button(settings, &SNAPPING))},
            {EntityScene(slider_row(settings, SettingSlider::TextSpeed))},
            {EntityScene(toggle_button(settings, &REDUCED_MOTION))},
            {volumes},
        ]
    })
}

fn slider_row(settings: &Settings, slider: SettingSlider) -> impl Scene + use<> {
    let (min, max) = slider.range();
    bsn! {
        Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(6.0), width: Val::Px(SLIDER_WIDTH), padding: {UiRect::vertical(Val::Px(6.0))} }
        Children [
            (
                {ui::styled_text(slider.label(settings), ui::theme::theme().surface_canvas.on, typography::BODY)}
                ui::component(SliderLabel(slider))
            ),
            (
                {ui::slider(slider.value(settings), min, max)}
                on(move |changed: On<ValueChange<f32>>, mut settings: ResMut<Settings>| {
                    slider.set(&mut settings, changed.value);
                })
                Children [
                    ( {ui::slider_track()}
                      Children [ {EntityScene(ui::slider_range())}, {EntityScene(ui::slider_thumb())} ]
                    )
                ]
            ),
        ]
    }
}

impl SettingSlider {
    fn label(self, settings: &Settings) -> String {
        match self {
            SettingSlider::TextSpeed => format!(
                "text speed {} letters a second",
                settings.letters_per_second().0 as u32
            ),
            SettingSlider::Volume(fader) => format!(
                "{} volume {}%",
                fader_name(fader),
                (settings.volume(fader).0 * 100.0).round() as u32
            ),
        }
    }

    fn value(self, settings: &Settings) -> f32 {
        match self {
            SettingSlider::TextSpeed => settings.letters_per_second().0,
            SettingSlider::Volume(fader) => settings.volume(fader).0,
        }
    }

    fn range(self) -> (f32, f32) {
        match self {
            SettingSlider::TextSpeed => (LettersPerSecond::SLOWEST.0, LettersPerSecond::FASTEST.0),
            SettingSlider::Volume(_) => (AudioVolume::SILENT.0, AudioVolume::FULL.0),
        }
    }

    fn set(self, settings: &mut Settings, value: f32) {
        match self {
            SettingSlider::TextSpeed => {
                settings.set_letters_per_second(LettersPerSecond(value.round()));
            }
            SettingSlider::Volume(fader) => {
                settings.set_volume(fader, AudioVolume((value * 100.0).round() / 100.0));
            }
        }
    }
}

fn fader_name(fader: AudioFader) -> &'static str {
    match fader {
        AudioFader::Master => "master",
        AudioFader::Category(AudioCategory::Music) => "music",
        AudioFader::Category(AudioCategory::Voice) => "voice",
        AudioFader::Category(AudioCategory::Effects) => "effects",
    }
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

fn sync_labels(world: &mut World) {
    if !world.is_resource_changed::<Settings>() {
        return;
    }
    let buttons: Vec<(&'static Toggle, Vec<Entity>)> = world
        .query::<(&ToggleButton, &Children)>()
        .iter(world)
        .map(|(button, children)| (button.0, children.iter().collect()))
        .collect();
    let labels: Vec<(Entity, String)> = world
        .query::<(Entity, &SliderLabel)>()
        .iter(world)
        .map(|(entity, label)| (entity, label.0.label(world.resource::<Settings>())))
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
