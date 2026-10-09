use bevy::prelude::*;
use bevy::scene::EntityScene;
use ui::button::intent as button_intent;
use ui::tokens::typography;
use ui::{Activate, ButtonSize, ValueChange, button_styled};

use crate::core::audio::mix::{AudioCategory, AudioFader, AudioVolume};
use crate::core::render::transition::ScreenTransition;
use crate::systems::hud::{HudAudience, LettersPerSecond, Settings, Window};
use crate::systems::input::map::InputAction;

const SLIDER_WIDTH: f32 = 240.0;
const FADER_NAME_WIDTH: f32 = 80.0;

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

const FADERS: [AudioFader; 5] = [
    AudioFader::Master,
    AudioFader::Category(AudioCategory::Music),
    AudioFader::Category(AudioCategory::Ambience),
    AudioFader::Category(AudioCategory::Effects),
    AudioFader::Category(AudioCategory::Voice),
];

#[derive(Component, Clone)]
struct ToggleButton(&'static Toggle);

#[derive(Component, Clone, Default)]
struct TextSpeedLabel;

fn content(world: &World) -> Box<dyn Scene> {
    let settings = world.resource::<Settings>();
    Box::new(bsn! {
        Node { flex_direction: FlexDirection::Column, align_items: AlignItems::Start, row_gap: Val::Px(6.0) }
        Children [
            {EntityScene(toggle_button(settings, &SNAPPING))},
            {EntityScene(text_speed_row(settings))},
            {EntityScene(toggle_button(settings, &REDUCED_MOTION))},
            {EntityScene(transition_row(settings))},
            {EntityScene(audio_section(settings))},
        ]
    })
}

fn text_speed_row(settings: &Settings) -> impl Scene + use<> {
    bsn! {
        Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(6.0), width: Val::Px(SLIDER_WIDTH), padding: {UiRect::vertical(Val::Px(6.0))} }
        Children [
            (
                {ui::styled_text(text_speed_label(settings), ui::theme::theme().surface_canvas.on, typography::BODY)}
                TextSpeedLabel
            ),
            {EntityScene(slider(settings, SettingSlider::TextSpeed))},
        ]
    }
}

fn transition_row(settings: &Settings) -> impl Scene + use<> {
    let choices: Vec<Box<dyn Scene>> = ScreenTransition::ALL
        .into_iter()
        .map(|transition| -> Box<dyn Scene> {
            Box::new(ui::dropdown_item(transition.label(), transition.label()))
        })
        .collect();
    bsn! {
        Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(6.0), width: Val::Px(SLIDER_WIDTH), padding: {UiRect::vertical(Val::Px(6.0))} }
        Children [
            {EntityScene(ui::styled_text("area transition", ui::theme::theme().surface_canvas.on, typography::BODY))},
            (
                {ui::dropdown(settings.screen_transition().label())}
                on(|picked: On<ValueChange<String>>, mut settings: ResMut<Settings>| {
                    if let Some(transition) = ScreenTransition::ALL
                        .into_iter()
                        .find(|transition| transition.label() == picked.value)
                    {
                        settings.set_screen_transition(transition);
                    }
                })
                Children [
                    {EntityScene(ui::dropdown_trigger())},
                    ( {ui::dropdown_content()} Children [ {choices} ] ),
                ]
            ),
        ]
    }
}

fn audio_section(settings: &Settings) -> impl Scene + use<> {
    let faders: Vec<Box<dyn Scene>> = FADERS
        .into_iter()
        .map(|fader| -> Box<dyn Scene> { Box::new(fader_row(settings, fader)) })
        .collect();
    bsn! {
        Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(4.0), width: {Val::Px(SLIDER_WIDTH + FADER_NAME_WIDTH)}, padding: {UiRect::top(Val::Px(6.0))} }
        Children [
            {EntityScene(ui::styled_text("Audio", ui::theme::theme().surface_canvas.on, typography::NAME))},
            {faders},
        ]
    }
}

fn fader_row(settings: &Settings, fader: AudioFader) -> impl Scene + use<> {
    bsn! {
        Node { flex_direction: FlexDirection::Row, align_items: AlignItems::Center }
        Children [
            (
                {ui::styled_text(fader_name(fader), ui::theme::theme().surface_canvas.on, typography::BODY)}
                Node { width: Val::Px(FADER_NAME_WIDTH) }
            ),
            (
                Node { flex_grow: 1.0 }
                Children [ {EntityScene(slider(settings, SettingSlider::Volume(fader)))} ]
            ),
        ]
    }
}

fn slider(settings: &Settings, slider: SettingSlider) -> impl Scene + use<> {
    let (min, max) = slider.range();
    bsn! {
        {ui::slider(slider.value(settings), min, max)}
        on(move |changed: On<ValueChange<f32>>, mut settings: ResMut<Settings>| {
            slider.set(&mut settings, changed.value);
        })
        Children [
            ( {ui::slider_track()}
              Children [ {EntityScene(ui::slider_range())}, {EntityScene(ui::slider_thumb())} ]
            )
        ]
    }
}

fn text_speed_label(settings: &Settings) -> String {
    format!(
        "text speed {} letters a second",
        settings.letters_per_second().0 as u32
    )
}

impl SettingSlider {
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
        AudioFader::Master => "Master",
        AudioFader::Category(AudioCategory::Music) => "Music",
        AudioFader::Category(AudioCategory::Ambience) => "Ambience",
        AudioFader::Category(AudioCategory::Effects) => "Effects",
        AudioFader::Category(AudioCategory::Voice) => "Voice",
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
    let speed = text_speed_label(world.resource::<Settings>());
    let labels: Vec<(Entity, String)> = world
        .query_filtered::<Entity, With<TextSpeedLabel>>()
        .iter(world)
        .map(|entity| (entity, speed.clone()))
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
