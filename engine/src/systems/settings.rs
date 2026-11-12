use std::collections::BTreeMap;
use std::iter;
use ui::egui;
use ui::theme;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use strum::VariantArray;
use ui::egui::Ui;
use ui::{controls, sheet};

use crate::core::audio::mix::{AudioCategory, AudioFader, AudioMix, AudioVolume};
use crate::core::math::{Pos, Size};
use crate::core::os::Profile;
use crate::core::render::transition::{ScreenTransition, ScreenTransitionPreference};
use crate::core::render::video::{self, VideoPreferences};
use crate::core::render::{CameraPreferences, CameraRig};
use crate::core::text::{LettersPerSecond, Typing};
use crate::core::units::{Seconds, WindowPx};
use crate::systems::hud::HudWindow;
use crate::systems::input::map::{InputAction, KeyGesture};
use crate::systems::interface::InterfaceIcon;
use crate::systems::skill::HotbarSlot;

const WINDOW_SIZE: egui::Vec2 = egui::Vec2::new(400.0, 410.0);
const SLIDER_WIDTH: f32 = 150.0;

pub(crate) fn plugin(app: &mut App) {
    let settings = UserSettings::load(app.world().resource::<Profile>());
    app.insert_resource(settings)
        .add_systems(Update, (persist, apply, apply_video, pace_typing));
}

/// Saves `video` as the player's video preferences in `profile`, keeping their other settings.
pub fn save_video(profile: &Profile, video: VideoPreferences) {
    let mut settings = UserSettings::load(profile);
    settings.video = video;
    settings.save(profile);
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
    #[serde(default)]
    camera: CameraSettings,
    #[serde(default)]
    video: VideoPreferences,
    #[serde(default)]
    crafting: CraftingSettings,
}

#[derive(Serialize, Deserialize, Clone, Copy)]
pub(crate) struct Placement {
    pub pos: Pos<WindowPx>,
    pub size: Size<WindowPx>,
}

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

    /// The shape as far as a bar of `slots` fills it: no wider than it has slots, and no more rows
    /// than they fill.
    pub(crate) fn within(self, slots: u32) -> HotbarShape {
        let columns = self.columns.clamp(1, slots.max(1));
        HotbarShape {
            columns,
            rows: self.rows.clamp(1, slots.max(1).div_ceil(columns)),
        }
    }

    pub(crate) fn slots(self) -> u32 {
        self.columns * self.rows
    }
}

/// What key a hotbar slot answers to: the game's own for the slot, none, or one the player chose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HotbarKey {
    Default,
    Unbound,
    Custom(KeyGesture),
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
        let typing = world.get_resource::<Typing>().copied();
        let turn_ease = world
            .get_resource::<CameraRig>()
            .map(|rig| rig.framing().turn_ease);
        sheet::scroll(ui, egui::ScrollArea::vertical().auto_shrink(false), |ui| {
            show_settings(
                ui,
                &mut world.resource_mut::<UserSettings>(),
                typing,
                turn_ease,
            )
        });
    }
}

fn show_settings(
    ui: &mut Ui,
    settings: &mut Mut<UserSettings>,
    typing: Option<Typing>,
    turn_ease: Option<Seconds>,
) {
    ui.spacing_mut().slider_width = SLIDER_WIDTH;
    sheet::section(ui, "Interface");
    let mut snapping = settings.snapping_enabled();
    if ui.checkbox(&mut snapping, "UI snapping").changed() {
        settings.toggle_snapping();
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
    let mut confirms = settings.confirms_risky_crafts();
    if ui
        .checkbox(&mut confirms, "Confirm crafts that may fail")
        .on_hover_text("A craft that may fail asks before it spends its ingredients")
        .changed()
    {
        settings.toggle_risky_craft_confirm();
    }
    ui.add_space(4.0);
    controls::form(ui, ("settings", "interface"), |ui| {
        if let Some(typing) = typing {
            let label = ui.label("Text speed");
            text_speed_slider(ui, settings, typing, label.id);
            ui.end_row();
        }
        let label = ui.label("Area transition");
        let current = settings.screen_transition();
        if let Some(transition) = controls::variant_picker(ui, "area transition", current, label.id)
        {
            settings.set_screen_transition(transition);
        }
        ui.end_row();
    });
    ui.add_space(8.0);
    sheet::section(ui, "Camera");
    controls::form(ui, ("settings", "camera"), |ui| {
        if let Some(turn_ease) = turn_ease {
            let label = ui.label("Camera smoothing");
            smoothing_slider(ui, settings, turn_ease, label.id);
            ui.end_row();
        }
    });
    ui.add_space(8.0);
    sheet::section(ui, "Video");
    show_video(ui, settings);
    ui.add_space(8.0);
    sheet::section(ui, "Audio");
    controls::form(ui, ("settings", "audio"), |ui| {
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

fn show_video(ui: &mut Ui, settings: &mut Mut<UserSettings>) {
    let mut video = settings.video;
    video::controls(ui, &mut video);
    if video != settings.video {
        settings.video = video;
    }
}

fn text_speed_slider(
    ui: &mut Ui,
    settings: &mut Mut<UserSettings>,
    typing: Typing,
    label: egui::Id,
) {
    let mut speed = typing.pace_for(settings.text.letters_per_second).letters.0;
    ui.horizontal(|ui| {
        let range = typing.slowest.0..=typing.fastest.0;
        if controls::slider(ui, &mut speed, range)
            .labelled_by(label)
            .changed()
        {
            settings.text.letters_per_second = Some(LettersPerSecond(speed.round()));
        }
        ui.label(theme::faint(format!("{:.0} letters/s", speed.round())));
    });
}

fn smoothing_slider(ui: &mut Ui, settings: &mut Mut<UserSettings>, game: Seconds, label: egui::Id) {
    let mut ease = smoothing(settings.camera.turn_ease, game).0;
    ui.horizontal(|ui| {
        if controls::slider(ui, &mut ease, SMOOTHINGS.0.0..=SMOOTHINGS.1.0)
            .labelled_by(label)
            .changed()
        {
            settings.camera.turn_ease = Some(Seconds((ease * 100.0).round() / 100.0));
        }
        let shown = if ease > 0.0 {
            format!("{ease:.2} s")
        } else {
            "off".to_owned()
        };
        ui.label(theme::faint(shown));
    });
}

fn fader_slider(ui: &mut Ui, settings: &mut Mut<UserSettings>, fader: AudioFader, label: egui::Id) {
    let mut volume = settings.volume(fader).0;
    ui.horizontal(|ui| {
        let range = AudioVolume::SILENT.0..=AudioVolume::FULL.0;
        if controls::slider(ui, &mut volume, range)
            .labelled_by(label)
            .changed()
        {
            settings.set_volume(fader, AudioVolume((volume * 100.0).round() / 100.0));
        }
        ui.label(theme::faint(format!("{:.0}%", volume * 100.0)));
    });
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

    pub(crate) fn confirms_risky_crafts(&self) -> bool {
        !self.crafting.unconfirmed
    }

    pub(crate) fn toggle_risky_craft_confirm(&mut self) {
        self.crafting.unconfirmed = !self.crafting.unconfirmed;
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

    pub(crate) fn hotbar_key(&self, slot: HotbarSlot) -> HotbarKey {
        match self.ui.hotbar.keys.get(&slot) {
            None => HotbarKey::Default,
            Some(None) => HotbarKey::Unbound,
            Some(Some(gesture)) => HotbarKey::Custom(*gesture),
        }
    }

    pub(crate) fn set_hotbar_key(&mut self, slot: HotbarSlot, key: HotbarKey) {
        match key {
            HotbarKey::Default => self.ui.hotbar.keys.remove(&slot),
            HotbarKey::Unbound => self.ui.hotbar.keys.insert(slot, None),
            HotbarKey::Custom(gesture) => self.ui.hotbar.keys.insert(slot, Some(gesture)),
        };
    }

    fn load(profile: &Profile) -> UserSettings {
        let mut settings: UserSettings = profile
            .settings()
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        settings.ui.snap = settings.ui.snap.filter(|snap| snap.0 > 0.0);
        settings
    }

    fn save(&self, profile: &Profile) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            profile.save_settings(&json);
        }
    }
}

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
    /// A slot absent keeps the game's key; one listed with no gesture has none.
    keys: BTreeMap<HotbarSlot, Option<KeyGesture>>,
}

fn default_snap() -> Option<WindowPx> {
    Some(DEFAULT_SNAP)
}

#[derive(Serialize, Deserialize, Default)]
struct TextSettings {
    /// None keeps the game's own pace.
    #[serde(default)]
    letters_per_second: Option<LettersPerSecond>,
}

#[derive(Serialize, Deserialize, Default)]
struct CraftingSettings {
    /// Whether a craft that may fail goes ahead unasked.
    #[serde(default)]
    unconfirmed: bool,
}

#[derive(Serialize, Deserialize, Default)]
struct SkillSettings {
    #[serde(default)]
    quick_cast: bool,
}

/// The least and most a player may smooth the camera's turns and tilts by: none at all, up to
/// the ease of a slow cinematic camera.
const SMOOTHINGS: (Seconds, Seconds) = (Seconds(0.0), Seconds(0.5));

#[derive(Serialize, Deserialize, Default)]
struct CameraSettings {
    /// None keeps the game's own smoothing.
    #[serde(default)]
    turn_ease: Option<Seconds>,
}

/// The ease the camera turns with for a player who `chose` one, or chose none.
fn smoothing(chose: Option<Seconds>, game: Seconds) -> Seconds {
    chose.map_or(game, within_smoothings)
}

fn within_smoothings(chose: Seconds) -> Seconds {
    Seconds(chose.0.clamp(SMOOTHINGS.0.0, SMOOTHINGS.1.0))
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
fn persist(settings: Res<UserSettings>, profile: Res<Profile>) {
    if settings.is_changed() {
        settings.save(&profile);
    }
}

fn apply(
    settings: Res<UserSettings>,
    mut audio_mix: ResMut<AudioMix>,
    mut transition: ResMut<ScreenTransitionPreference>,
    mut camera: ResMut<CameraPreferences>,
) {
    if !settings.is_changed() {
        return;
    }
    *audio_mix = settings.sound;
    transition.0 = settings.screen.transition;
    camera.turn_ease = settings.camera.turn_ease.map(within_smoothings);
}

/// Lines type at the game's pace, or the one its player chose within the game's bounds.
fn pace_typing(settings: Res<UserSettings>, typing: Option<Res<Typing>>, mut commands: Commands) {
    let Some(typing) = typing else {
        return;
    };
    if settings.is_changed() || typing.is_changed() {
        commands.insert_resource(typing.pace_for(settings.text.letters_per_second));
    }
}

fn apply_video(settings: Res<UserSettings>, mut video: ResMut<VideoPreferences>) {
    if settings.is_changed() {
        video.set_if_neq(settings.video);
    }
}
