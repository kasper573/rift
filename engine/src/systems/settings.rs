use std::collections::BTreeMap;
use std::iter;
use ui::egui;

use bevy::prelude::*;
use bevy::window::{MonitorSelection, PresentMode, PrimaryWindow, WindowMode};
use serde::{Deserialize, Serialize};
use strum::VariantArray;
use ui::egui::Ui;
use ui::{controls, face, sheet};

use crate::core::audio::mix::{AudioCategory, AudioFader, AudioMix, AudioVolume};
use crate::core::geometry::Degrees;
use crate::core::math::{Pos, Size, WindowPx};
use crate::core::platform::{ClientPlatform, Platform};
use crate::core::render::transition::{ScreenTransition, ScreenTransitionPreference};
use crate::core::render::video::{AmbientOcclusionQuality, VideoPreferences};
use crate::core::render::{CameraPreferences, FullPicture};
use crate::core::text::LettersPerSecond;
use crate::systems::hud::HudWindow;
use crate::systems::input::map::{InputAction, KeyGesture};
use crate::systems::interface::InterfaceIcon;
use crate::systems::skill::HotbarSlot;

const WINDOW_SIZE: egui::Vec2 = egui::Vec2::new(400.0, 410.0);
const SLIDER_WIDTH: f32 = 150.0;
const LABEL_WIDTH: f32 = 100.0;
const FORM_SPACING: egui::Vec2 = egui::Vec2::new(16.0, 8.0);

pub(crate) fn plugin(app: &mut App) {
    let settings = UserSettings::load(&*app.world().resource::<ClientPlatform>().0);
    app.insert_resource(settings)
        .add_systems(Update, (persist, apply, apply_video));
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
    window: WindowSettings,
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
        let offered = VideoOffered {
            full_picture: world.resource::<FullPicture>().0,
            window: world.resource::<ClientPlatform>().0.owns_window(),
        };
        sheet::scroll(ui, egui::ScrollArea::vertical().auto_shrink(false), |ui| {
            show_settings(ui, &mut world.resource_mut::<UserSettings>(), offered)
        });
    }
}

/// The video settings this client can honour beside those every client can; the rest show, locked.
#[derive(Clone, Copy)]
struct VideoOffered {
    /// The light shafts and the ambient occlusion, which only the full picture draws.
    full_picture: bool,
    window: bool,
}

fn show_settings(ui: &mut Ui, settings: &mut Mut<UserSettings>, offered: VideoOffered) {
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
    ui.add_space(4.0);
    form(ui, "interface", |ui| {
        let label = ui.label("Text speed");
        text_speed_slider(ui, settings, label.id);
        ui.end_row();
        let label = ui.label("Area transition");
        let current = settings.screen_transition();
        if let Some(transition) = picker(ui, "area transition", current, label.id) {
            settings.set_screen_transition(transition);
        }
        ui.end_row();
    });
    ui.add_space(8.0);
    sheet::section(ui, "Camera");
    form(ui, "camera", |ui| {
        let label = ui.label("Turn speed");
        let mut turn_speed = settings.camera.turn_speed.0;
        if ui
            .add(
                egui::Slider::new(&mut turn_speed, TURN_SPEEDS.0..=TURN_SPEEDS.1)
                    .suffix("°/s")
                    .integer(),
            )
            .labelled_by(label.id)
            .changed()
        {
            settings.camera.turn_speed = Degrees(turn_speed);
        }
        ui.end_row();
    });
    ui.add_space(8.0);
    sheet::section(ui, "Video");
    show_video(ui, settings, offered);
    ui.add_space(8.0);
    sheet::section(ui, "Audio");
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

fn show_video(ui: &mut Ui, settings: &mut Mut<UserSettings>, offered: VideoOffered) {
    let mut fullscreen = settings.window.fullscreen && offered.window;
    if supported_only(ui, offered.window, |ui| {
        ui.checkbox(&mut fullscreen, "Fullscreen")
    })
    .changed()
    {
        settings.window.fullscreen = fullscreen;
    }
    let mut vsync = settings.window.vsync && offered.window;
    if supported_only(ui, offered.window, |ui| {
        ui.checkbox(&mut vsync, "VSync").on_hover_text(
            "Each frame waits for the display's refresh: less heat and power, a little more delay",
        )
    })
    .changed()
    {
        settings.window.vsync = vsync;
    }
    let mut bloom = settings.video.bloom;
    if ui
        .checkbox(&mut bloom, "Bloom")
        .on_hover_text("What the light catches glows over its rim")
        .changed()
    {
        settings.video.bloom = bloom;
    }
    let mut light_shafts = settings.video.light_shafts && offered.full_picture;
    if supported_only(ui, offered.full_picture, |ui| {
        ui.checkbox(&mut light_shafts, "Light shafts")
            .on_hover_text("The sun's rays show in the air")
    })
    .changed()
    {
        settings.video.light_shafts = light_shafts;
    }
    ui.add_space(4.0);
    form(ui, "video", |ui| {
        let label = ui.label("Shadows");
        if let Some(shadows) = picker(ui, "shadows", settings.video.shadows, label.id) {
            settings.video.shadows = shadows;
        }
        ui.end_row();
        let label = ui.add_enabled(offered.full_picture, egui::Label::new("Ambient occlusion"));
        let current = if offered.full_picture {
            settings.video.ambient_occlusion
        } else {
            AmbientOcclusionQuality::Off
        };
        if let Some(occlusion) = supported_only(ui, offered.full_picture, |ui| {
            picker(ui, "ambient occlusion", current, label.id)
        }) {
            settings.video.ambient_occlusion = occlusion;
        }
        ui.end_row();
        let label = ui.label("Anti-aliasing");
        let current = settings.video.anti_aliasing;
        if let Some(anti_aliasing) = picker(ui, "anti-aliasing", current, label.id) {
            settings.video.anti_aliasing = anti_aliasing;
        }
        ui.end_row();
    });
}

/// `add`'s controls, dimmed and locked with the reason on hover where the platform can't honour
/// them, in which case they must show what is drawn: off.
fn supported_only<R>(ui: &mut Ui, supported: bool, add: impl FnOnce(&mut Ui) -> R) -> R {
    let shown = ui.add_enabled_ui(supported, add);
    shown
        .response
        .on_disabled_hover_text("Not supported on this platform");
    shown.inner
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
    ui.horizontal(|ui| {
        let range = LettersPerSecond::SLOWEST.0..=LettersPerSecond::FASTEST.0;
        if controls::slider(ui, &mut speed, range)
            .labelled_by(label)
            .changed()
        {
            settings.set_letters_per_second(LettersPerSecond(speed.round()));
        }
        ui.label(face::faint(format!("{:.0} letters/s", speed.round())));
    });
}

/// A choice among every variant of `T`, answering the one picked when it isn't `current`.
fn picker<T: VariantArray + AsRef<str> + Copy + PartialEq>(
    ui: &mut Ui,
    id: &str,
    current: T,
    label: egui::Id,
) -> Option<T> {
    let mut picked = current;
    controls::combo(id)
        .selected_text(current.as_ref())
        .show_ui(ui, |ui| {
            for &variant in T::VARIANTS {
                ui.selectable_value(&mut picked, variant, variant.as_ref());
            }
        })
        .response
        .labelled_by(label);
    (picked != current).then_some(picked)
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
        ui.label(face::faint(format!("{:.0}%", volume * 100.0)));
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

    pub(crate) fn letters_per_second(&self) -> LettersPerSecond {
        self.text.letters_per_second
    }

    pub(crate) fn set_letters_per_second(&mut self, letters_per_second: LettersPerSecond) {
        self.text.letters_per_second = letters_per_second.clamped();
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
    /// A slot absent keeps the game's key; one listed with no gesture has none.
    keys: BTreeMap<HotbarSlot, Option<KeyGesture>>,
}

fn default_snap() -> Option<WindowPx> {
    Some(DEFAULT_SNAP)
}

#[derive(Serialize, Deserialize, Default)]
struct TextSettings {
    #[serde(default)]
    letters_per_second: LettersPerSecond,
}

#[derive(Serialize, Deserialize, Default)]
struct SkillSettings {
    #[serde(default)]
    quick_cast: bool,
}

/// How slow or fast the camera may turn, in degrees a second.
const TURN_SPEEDS: (f32, f32) = (45.0, 720.0);

#[derive(Serialize, Deserialize)]
struct CameraSettings {
    #[serde(default = "default_turn_speed")]
    turn_speed: Degrees,
}

impl Default for CameraSettings {
    fn default() -> CameraSettings {
        CameraSettings {
            turn_speed: default_turn_speed(),
        }
    }
}

fn default_turn_speed() -> Degrees {
    CameraPreferences::default().turn_speed
}

/// The window's screen mode and refresh, on a platform where the window is the client's own.
#[derive(Serialize, Deserialize, Default)]
struct WindowSettings {
    #[serde(default)]
    fullscreen: bool,
    /// Off unless asked for, so frames go out without waiting on the display's refresh: a desktop
    /// may hold that wait up, as some do for tens of milliseconds at every mouse press and
    /// release, and the whole frame would wait too.
    #[serde(default)]
    vsync: bool,
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
    mut speed: ResMut<LettersPerSecond>,
    mut audio_mix: ResMut<AudioMix>,
    mut transition: ResMut<ScreenTransitionPreference>,
    mut camera: ResMut<CameraPreferences>,
) {
    if !settings.is_changed() {
        return;
    }
    *speed = settings.text.letters_per_second;
    *audio_mix = settings.sound;
    transition.0 = settings.screen.transition;
    camera.turn_speed = Degrees(
        settings
            .camera
            .turn_speed
            .0
            .clamp(TURN_SPEEDS.0, TURN_SPEEDS.1),
    );
}

fn apply_video(
    settings: Res<UserSettings>,
    mut video: ResMut<VideoPreferences>,
    platform: Res<ClientPlatform>,
    window: Option<Single<&mut Window, With<PrimaryWindow>>>,
) {
    if !settings.is_changed() {
        return;
    }
    video.set_if_neq(settings.video);
    if let Some(mut window) = window.filter(|_| platform.0.owns_window()) {
        let mode = if settings.window.fullscreen {
            WindowMode::BorderlessFullscreen(MonitorSelection::Current)
        } else {
            WindowMode::Windowed
        };
        let present_mode = if settings.window.vsync {
            PresentMode::AutoVsync
        } else {
            PresentMode::AutoNoVsync
        };
        if window.mode != mode {
            window.mode = mode;
        }
        if window.present_mode != present_mode {
            window.present_mode = present_mode;
        }
    }
}
