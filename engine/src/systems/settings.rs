use std::collections::BTreeMap;
use std::iter;

use bevy::prelude::*;
use design_tokens::widgets;
use egui::Ui;
use serde::{Deserialize, Serialize};
use strum::VariantArray;

use crate::core::audio::mix::{AudioCategory, AudioFader, AudioMix, AudioVolume};
use crate::core::math::{Pos, Size, WindowPx};
use crate::core::platform::{ClientPlatform, Platform};
use crate::core::render::transition::{ScreenTransition, ScreenTransitionPreference};
use crate::core::text::{MotionPreference, TypewriterSpeed};
use crate::systems::hud::HudWindow;
use crate::systems::input::map::{InputAction, KeyGesture};
use crate::systems::interface::InterfaceIcon;

const WINDOW_SIZE: egui::Vec2 = egui::Vec2::new(400.0, 410.0);
const SLIDER_WIDTH: f32 = 150.0;
const LABEL_WIDTH: f32 = 100.0;
const FORM_SPACING: egui::Vec2 = egui::Vec2::new(16.0, 8.0);

pub(crate) fn plugin(app: &mut App) {
    let settings = UserSettings::load(&*app.world().resource::<ClientPlatform>().0);
    app.insert_resource(settings)
        .add_systems(Update, (persist, apply));
}

#[derive(Resource, Serialize, Deserialize, Default)]
pub(crate) struct UserSettings {
    #[serde(default)]
    ui: UiSettings,
    #[serde(default)]
    text: TextSettings,
    #[serde(default)]
    sound: AudioMix,
    #[serde(default)]
    screen: ScreenSettings,
    #[serde(default)]
    skills: SkillSettings,
}

#[derive(Serialize, Deserialize, Clone, Copy)]
pub(crate) struct Placement {
    pub pos: Pos<WindowPx>,
    pub size: Size<WindowPx>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, PartialOrd)]
pub(crate) struct LettersPerSecond(pub f32);

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HotbarShape {
    pub columns: u32,
    pub rows: u32,
}

impl HotbarShape {
    pub(crate) const DEFAULT: HotbarShape = HotbarShape {
        columns: 9,
        rows: 1,
    };

    pub(crate) fn slots(self) -> u32 {
        self.columns * self.rows
    }
}

pub struct SettingsWindow;

impl HudWindow for SettingsWindow {
    fn title(&self) -> &'static str {
        "Settings"
    }
    fn toggle(&self) -> InputAction {
        InputAction::ToggleSettings
    }
    fn icon(&self) -> InterfaceIcon {
        InterfaceIcon::Settings
    }
    fn size(&self) -> egui::Vec2 {
        WINDOW_SIZE
    }
    fn show(&self, ui: &mut Ui, world: &mut World) {
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                show_settings(ui, &mut world.resource_mut::<UserSettings>())
            });
    }
}

fn show_settings(ui: &mut Ui, settings: &mut Mut<UserSettings>) {
    ui.spacing_mut().slider_width = SLIDER_WIDTH;
    widgets::section(ui, "Interface");
    let mut snapping = settings.snapping_enabled();
    if ui.checkbox(&mut snapping, "UI snapping").changed() {
        settings.toggle_snapping();
    }
    let mut reduced = settings.reduced_motion();
    if ui.checkbox(&mut reduced, "Reduced motion").changed() {
        settings.toggle_reduced_motion();
    }
    let mut frame_rate = settings.frame_rate_shown();
    if ui.checkbox(&mut frame_rate, "Show FPS").changed() {
        settings.toggle_frame_rate();
    }
    let mut quick_cast = settings.quick_cast();
    if ui
        .checkbox(&mut quick_cast, "Quick cast skills")
        .on_hover_text("A skill is cast the moment it is used, at what the pointer rests on")
        .changed()
    {
        settings.toggle_quick_cast();
    }
    ui.add_space(4.0);
    form(ui, "interface", |ui| {
        let label = ui.label("Text speed");
        text_speed_slider(ui, settings, label.id);
        ui.end_row();
        let label = ui.label("Area transition");
        transition_picker(ui, settings, label.id);
        ui.end_row();
    });
    ui.add_space(8.0);
    widgets::section(ui, "Audio");
    form(ui, "audio", |ui| {
        for fader in iter::once(AudioFader::Master).chain(
            AudioCategory::VARIANTS
                .iter()
                .copied()
                .map(AudioFader::Category),
        ) {
            let label = ui.label(fader_name(fader));
            fader_slider(ui, settings, fader, label.id);
            ui.end_row();
        }
    });
}

fn form(ui: &mut Ui, id: &str, add: impl FnOnce(&mut Ui)) {
    egui::Grid::new(("settings", id))
        .num_columns(2)
        .min_col_width(LABEL_WIDTH)
        .spacing(FORM_SPACING)
        .show(ui, add);
}

fn text_speed_slider(ui: &mut Ui, settings: &mut Mut<UserSettings>, label: egui::Id) {
    let mut speed = settings.letters_per_second().0;
    let slider = egui::Slider::new(
        &mut speed,
        LettersPerSecond::SLOWEST.0..=LettersPerSecond::FASTEST.0,
    )
    .step_by(1.0)
    .fixed_decimals(0)
    .suffix(" letters/s");
    if widgets::slider(ui, slider).labelled_by(label).changed() {
        settings.set_letters_per_second(LettersPerSecond(speed.round()));
    }
}

fn transition_picker(ui: &mut Ui, settings: &mut Mut<UserSettings>, label: egui::Id) {
    let current = settings.screen_transition();
    let mut picked = current;
    widgets::combo_box("area transition")
        .selected_text(current.as_ref())
        .show_ui(ui, |ui| {
            for &transition in ScreenTransition::VARIANTS {
                ui.selectable_value(&mut picked, transition, transition.as_ref());
            }
        })
        .response
        .labelled_by(label);
    if picked != current {
        settings.set_screen_transition(picked);
    }
}

fn fader_slider(ui: &mut Ui, settings: &mut Mut<UserSettings>, fader: AudioFader, label: egui::Id) {
    let mut volume = settings.volume(fader).0;
    let slider = egui::Slider::new(&mut volume, AudioVolume::SILENT.0..=AudioVolume::FULL.0)
        .custom_formatter(|value, _| format!("{:.0}%", value * 100.0))
        .custom_parser(|typed| {
            typed
                .trim()
                .trim_end_matches('%')
                .parse::<f64>()
                .ok()
                .map(|percent| percent / 100.0)
        });
    if widgets::slider(ui, slider).labelled_by(label).changed() {
        settings.set_volume(fader, AudioVolume((volume * 100.0).round() / 100.0));
    }
}

fn fader_name(fader: AudioFader) -> &'static str {
    match fader {
        AudioFader::Master => "Master",
        AudioFader::Category(category) => category.into(),
    }
}

impl UserSettings {
    pub(crate) fn snapping_enabled(&self) -> bool {
        self.ui.snap.is_some()
    }

    pub(crate) fn snap(&self) -> Option<WindowPx> {
        self.ui.snap
    }

    pub(crate) fn toggle_snapping(&mut self) {
        self.ui.snap = match self.ui.snap {
            Some(_) => None,
            None => Some(DEFAULT_SNAP),
        };
    }

    pub(crate) fn letters_per_second(&self) -> LettersPerSecond {
        self.text.letters_per_second
    }

    pub(crate) fn set_letters_per_second(&mut self, letters_per_second: LettersPerSecond) {
        self.text.letters_per_second = letters_per_second.clamped();
    }

    pub(crate) fn reduced_motion(&self) -> bool {
        self.text.reduced_motion
    }

    pub(crate) fn toggle_reduced_motion(&mut self) {
        self.text.reduced_motion = !self.text.reduced_motion;
    }

    pub(crate) fn volume(&self, fader: AudioFader) -> AudioVolume {
        self.sound.level(fader)
    }

    pub(crate) fn set_volume(&mut self, fader: AudioFader, volume: AudioVolume) {
        self.sound.set(fader, volume);
    }

    pub(crate) fn screen_transition(&self) -> ScreenTransition {
        self.screen.transition
    }

    pub(crate) fn set_screen_transition(&mut self, transition: ScreenTransition) {
        self.screen.transition = transition;
    }

    pub(crate) fn frame_rate_shown(&self) -> bool {
        self.screen.frame_rate
    }

    pub(crate) fn toggle_frame_rate(&mut self) {
        self.screen.frame_rate = !self.screen.frame_rate;
    }

    pub(crate) fn quick_cast(&self) -> bool {
        self.skills.quick_cast
    }

    pub(crate) fn toggle_quick_cast(&mut self) {
        self.skills.quick_cast = !self.skills.quick_cast;
    }

    pub(crate) fn widget_pos(&self, id: &str) -> Option<Pos<WindowPx>> {
        self.ui.widgets.get(id).copied()
    }

    pub(crate) fn set_widget_pos(&mut self, id: &str, pos: Pos<WindowPx>) {
        self.ui.widgets.insert(id.to_owned(), pos);
    }

    pub(crate) fn window_placement(&self, id: &str) -> Option<Placement> {
        self.ui.windows.get(id).copied()
    }

    pub(crate) fn set_window_placement(&mut self, id: &str, placement: Placement) {
        self.ui.windows.insert(id.to_owned(), placement);
    }

    pub(crate) fn hotbar_shape(&self) -> HotbarShape {
        self.ui.hotbar.shape.unwrap_or(HotbarShape::DEFAULT)
    }

    pub(crate) fn set_hotbar_shape(&mut self, shape: HotbarShape) {
        self.ui.hotbar.shape = Some(shape);
    }

    /// `None` when the slot keeps the game's default key, `Some(None)` when its key was taken away.
    pub(crate) fn hotbar_key(&self, slot: u32) -> Option<Option<KeyGesture>> {
        self.ui.hotbar.keys.get(&slot).copied()
    }

    pub(crate) fn set_hotbar_key(&mut self, slot: u32, key: Option<KeyGesture>) {
        self.ui.hotbar.keys.insert(slot, key);
    }

    fn load(platform: &dyn Platform) -> UserSettings {
        let mut settings: UserSettings = platform
            .load(KEY)
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        settings.ui.snap = settings.ui.snap.filter(|snap| snap.0 > 0.0);
        settings.text.letters_per_second = settings.text.letters_per_second.clamped();
        settings
    }

    fn save(&self, platform: &dyn Platform) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            platform.save(KEY, &json);
        }
    }
}

impl LettersPerSecond {
    pub(crate) const SLOWEST: LettersPerSecond = LettersPerSecond(15.0);
    pub(crate) const FASTEST: LettersPerSecond = LettersPerSecond(120.0);

    fn clamped(self) -> LettersPerSecond {
        LettersPerSecond(self.0.clamp(Self::SLOWEST.0, Self::FASTEST.0))
    }
}

impl Default for LettersPerSecond {
    fn default() -> LettersPerSecond {
        LettersPerSecond(45.0)
    }
}

const KEY: &str = "rift.user_settings";
pub const DEFAULT_SNAP: WindowPx = WindowPx(16.0);

#[derive(Serialize, Deserialize)]
struct UiSettings {
    #[serde(default = "default_snap")]
    snap: Option<WindowPx>,
    #[serde(default)]
    widgets: BTreeMap<String, Pos<WindowPx>>,
    #[serde(default)]
    windows: BTreeMap<String, Placement>,
    #[serde(default)]
    hotbar: HotbarSettings,
}

impl Default for UiSettings {
    fn default() -> UiSettings {
        UiSettings {
            snap: default_snap(),
            widgets: BTreeMap::new(),
            windows: BTreeMap::new(),
            hotbar: HotbarSettings::default(),
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
struct HotbarSettings {
    #[serde(default)]
    shape: Option<HotbarShape>,
    #[serde(default)]
    keys: BTreeMap<u32, Option<KeyGesture>>,
}

fn default_snap() -> Option<WindowPx> {
    Some(DEFAULT_SNAP)
}

#[derive(Serialize, Deserialize, Default)]
struct TextSettings {
    #[serde(default)]
    letters_per_second: LettersPerSecond,
    #[serde(default)]
    reduced_motion: bool,
}

#[derive(Serialize, Deserialize, Default)]
struct SkillSettings {
    #[serde(default)]
    quick_cast: bool,
}

#[derive(Serialize, Deserialize, Default)]
struct ScreenSettings {
    #[serde(default)]
    transition: ScreenTransition,
    #[serde(default)]
    frame_rate: bool,
}

/// Saved on change rather than at each place that changes a setting. The initial load counts as a
/// change too, harmlessly writing back what was just read.
fn persist(settings: Res<UserSettings>, platform: Res<ClientPlatform>) {
    if settings.is_changed() {
        settings.save(&*platform.0);
    }
}

fn apply(
    settings: Res<UserSettings>,
    mut speed: ResMut<TypewriterSpeed>,
    mut motion: ResMut<MotionPreference>,
    mut audio_mix: ResMut<AudioMix>,
    mut transition: ResMut<ScreenTransitionPreference>,
) {
    if !settings.is_changed() {
        return;
    }
    *speed = TypewriterSpeed(Some(settings.text.letters_per_second.0));
    motion.reduced = settings.text.reduced_motion;
    *audio_mix = settings.sound;
    transition.0 = settings.screen.transition;
}
