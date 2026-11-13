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
use crate::core::units::{Percent, Seconds, WindowPx};
use crate::systems::hud;
use crate::systems::hud::menu::{MenuTab, MenuTabName};
use crate::systems::input::map::{InputAction, KeyGesture};
use crate::systems::interface::InterfaceIcon;
use crate::systems::minimap::MinimapReachStep;
use crate::systems::skill::HotbarSlot;

const SLIDER_WIDTH: f32 = 120.0;
const COLUMN_GAP: f32 = 16.0;
const ROW_GAP: f32 = 8.0;
const SECTION_GAP: f32 = 8.0;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Placed {
    Menu,
    Chat,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BarShape {
    pub columns: u32,
    pub rows: u32,
}

impl BarShape {
    pub(crate) const DEFAULT: BarShape = BarShape {
        columns: 9,
        rows: 1,
    };

    /// The shape as far as a bar of `slots` fills it: no wider than it has slots, and no more rows
    /// than they fill.
    pub(crate) fn within(self, slots: u32) -> BarShape {
        let columns = self.columns.clamp(1, slots.max(1));
        BarShape {
            columns,
            rows: self.rows.clamp(1, slots.max(1).div_ceil(columns)),
        }
    }

    pub(crate) fn slots(self) -> u32 {
        self.columns * self.rows
    }
}

/// What key a bar slot answers to: the game's own for the slot, none, or one the player chose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BarKey {
    Default,
    Unbound,
    Custom(KeyGesture),
}

pub struct SettingsMenuTab;

impl MenuTab for SettingsMenuTab {
    fn name(&self) -> MenuTabName {
        MenuTabName::Settings
    }
    fn icon(&self) -> InterfaceIcon {
        InterfaceIcon::Settings
    }
    fn opens(&self) -> InputAction {
        InputAction::OpenSettings
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
    ui.spacing_mut().item_spacing.x = COLUMN_GAP;
    ui.columns(2, |columns| {
        let [left, right] = columns else {
            return;
        };
        left.spacing_mut().item_spacing.x = ROW_GAP;
        right.spacing_mut().item_spacing.x = ROW_GAP;
        sheet::section(left, "Interface");
        show_interface(left, settings, typing);
        left.add_space(SECTION_GAP);
        sheet::section(left, "HUD");
        show_hud(left, settings);
        left.add_space(SECTION_GAP);
        sheet::section(left, "Camera");
        controls::form(left, ("settings", "camera"), |ui| {
            if let Some(turn_ease) = turn_ease {
                let label = ui.label("Camera smoothing");
                smoothing_slider(ui, settings, turn_ease, label.id);
                ui.end_row();
            }
        });
        sheet::section(right, "Chat");
        show_chat(right, settings);
        right.add_space(SECTION_GAP);
        sheet::section(right, "Video");
        show_video(right, settings);
        right.add_space(SECTION_GAP);
        sheet::section(right, "Audio");
        controls::form(right, ("settings", "audio"), |ui| {
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
    });
}

fn show_interface(ui: &mut Ui, settings: &mut Mut<UserSettings>, typing: Option<Typing>) {
    controls::form(ui, ("settings", "interface"), |ui| {
        let label = ui.label("Menu opens on");
        menu_tab_picker(ui, settings, label.id);
        ui.end_row();
        let mut snapping = settings.snapping_enabled();
        if controls::toggle(ui, "UI snapping", &mut snapping).changed() {
            settings.toggle_snapping();
        }
        ui.end_row();
        let mut frame_rate = settings.frame_rate_shown();
        if controls::toggle(ui, "Show FPS", &mut frame_rate).changed() {
            settings.toggle_frame_rate();
        }
        ui.end_row();
        let mut quick_cast = settings.quick_cast();
        if controls::toggle(ui, "Quick cast skills", &mut quick_cast)
            .on_hover_text("A skill is cast the moment it is used, at what the pointer rests on")
            .changed()
        {
            settings.toggle_quick_cast();
        }
        ui.end_row();
        let mut confirms = settings.confirms_risky_crafts();
        if controls::toggle(ui, "Confirm crafts that may fail", &mut confirms)
            .on_hover_text("A craft that may fail asks before it spends its materials")
            .changed()
        {
            settings.toggle_risky_craft_confirm();
        }
        ui.end_row();
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
}

fn show_hud(ui: &mut Ui, settings: &mut Mut<UserSettings>) {
    controls::form(ui, ("settings", "hud"), |ui| {
        let mut turns = settings.minimap_turns();
        if controls::toggle(ui, "Minimap turns with the camera", &mut turns)
            .on_hover_text("Up on the map is where the camera looks, and north rides its rim")
            .changed()
        {
            settings.toggle_minimap_turning();
        }
        ui.end_row();
        let label = ui.label("Quest tracker backing");
        tracker_backing_slider(ui, settings, label.id);
        ui.end_row();
        let label = ui.label("HUD places");
        if controls::button(ui, "Reset")
            .labelled_by(label.id)
            .on_hover_text("Every piece of the HUD goes back to where it started, chat too")
            .clicked()
        {
            settings.reset_hud();
        }
        ui.end_row();
    });
}

fn show_chat(ui: &mut Ui, settings: &mut Mut<UserSettings>) {
    controls::form(ui, ("settings", "chat"), |ui| {
        let mut chat = settings.chat_shown();
        if controls::toggle(ui, "Show chat", &mut chat)
            .on_hover_text("Off, chat leaves the screen and its key does nothing")
            .changed()
        {
            settings.toggle_chat();
        }
        ui.end_row();
        let label = ui.label("Chat background");
        chat_background_slider(ui, settings, label.id);
        ui.end_row();
        let label = ui.label("Chat text size");
        chat_text_slider(ui, settings, label.id);
        ui.end_row();
    });
}

fn menu_tab_picker(ui: &mut Ui, settings: &mut Mut<UserSettings>, label: egui::Id) {
    let chosen = settings.menu_opens_on();
    let mut picked = chosen;
    controls::combo("menu opens on")
        .selected_text(chosen.map_or(LAST_USED_TAB, MenuTabName::title))
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut picked, None, LAST_USED_TAB);
            ui.separator();
            for &tab in MenuTabName::VARIANTS {
                ui.selectable_value(&mut picked, Some(tab), tab.title());
            }
        })
        .response
        .labelled_by(label);
    if picked != chosen {
        settings.ui.menu.opens_on = picked;
    }
}

const LAST_USED_TAB: &str = "The tab I last used";

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

fn chat_background_slider(ui: &mut Ui, settings: &mut Mut<UserSettings>, label: egui::Id) {
    let mut background = settings.chat_background().0;
    ui.horizontal(|ui| {
        if controls::slider(ui, &mut background, 0.0..=Percent::WHOLE.0)
            .labelled_by(label)
            .changed()
        {
            settings.ui.chat.background = Some(Percent(background.round()));
        }
        ui.label(theme::faint(format!("{:.0}%", background.round())));
    });
}

fn tracker_backing_slider(ui: &mut Ui, settings: &mut Mut<UserSettings>, label: egui::Id) {
    let mut backing = settings.tracker_backing().0;
    ui.horizontal(|ui| {
        if controls::slider(ui, &mut backing, 0.0..=Percent::WHOLE.0)
            .labelled_by(label)
            .changed()
        {
            settings.ui.tracker_backing = Percent(backing.round());
        }
        let shown = if backing > 0.0 {
            format!("{:.0}%", backing.round())
        } else {
            "None".to_owned()
        };
        ui.label(theme::faint(shown));
    });
}

fn chat_text_slider(ui: &mut Ui, settings: &mut Mut<UserSettings>, label: egui::Id) {
    let mut size = settings.chat_text_size().0;
    ui.horizontal(|ui| {
        let (_, smallest, largest) = CHAT_TEXT_SIZES;
        if controls::slider(ui, &mut size, smallest.0..=largest.0)
            .labelled_by(label)
            .changed()
        {
            settings.ui.chat.text_size = Some(WindowPx(size.round()));
        }
        ui.label(theme::faint(format!("{:.0} px", size.round())));
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

    pub(crate) fn reset_hud(&mut self) {
        self.ui.widgets.retain(|name, _| !hud::is_piece(name));
        self.ui.chat.placement = None;
        self.ui.player_bar.shape = None;
    }

    pub(crate) fn placement(&self, of: Placed) -> Option<Placement> {
        match of {
            Placed::Menu => self.ui.menu.placement,
            Placed::Chat => self.ui.chat.placement,
        }
    }

    pub(crate) fn set_placement(&mut self, of: Placed, placement: Placement) {
        match of {
            Placed::Menu => self.ui.menu.placement = Some(placement),
            Placed::Chat => self.ui.chat.placement = Some(placement),
        }
    }

    pub(crate) fn menu_opens_on(&self) -> Option<MenuTabName> {
        self.ui.menu.opens_on
    }

    pub(crate) fn minimap_turns(&self) -> bool {
        self.ui.minimap.turns_with_camera
    }

    pub(crate) fn toggle_minimap_turning(&mut self) {
        self.ui.minimap.turns_with_camera = !self.ui.minimap.turns_with_camera;
    }

    pub(crate) fn minimap_reach(&self) -> Option<MinimapReachStep> {
        self.ui.minimap.reach
    }

    pub(crate) fn set_minimap_reach(&mut self, step: MinimapReachStep) {
        self.ui.minimap.reach = Some(step);
    }

    pub(crate) fn tracker_backing(&self) -> Percent {
        self.ui.tracker_backing
    }

    pub(crate) fn chat_shown(&self) -> bool {
        !self.ui.chat.hidden
    }

    pub(crate) fn toggle_chat(&mut self) {
        self.ui.chat.hidden = !self.ui.chat.hidden;
    }

    pub(crate) fn chat_background(&self) -> Percent {
        self.ui.chat.background.unwrap_or(CHAT_BACKGROUND)
    }

    pub(crate) fn chat_text_size(&self) -> WindowPx {
        let size = self.ui.chat.text_size.unwrap_or(CHAT_TEXT_SIZES.0);
        WindowPx(size.0.clamp(CHAT_TEXT_SIZES.1.0, CHAT_TEXT_SIZES.2.0))
    }

    pub(crate) fn bar_shape(&self) -> BarShape {
        self.ui.player_bar.shape.unwrap_or(BarShape::DEFAULT)
    }

    pub(crate) fn set_bar_shape(&mut self, shape: BarShape) {
        self.ui.player_bar.shape = Some(shape);
    }

    pub(crate) fn bar_key(&self, slot: HotbarSlot) -> BarKey {
        match self.ui.player_bar.keys.get(&slot) {
            None => BarKey::Default,
            Some(None) => BarKey::Unbound,
            Some(Some(gesture)) => BarKey::Custom(*gesture),
        }
    }

    pub(crate) fn set_bar_key(&mut self, slot: HotbarSlot, key: BarKey) {
        match key {
            BarKey::Default => self.ui.player_bar.keys.remove(&slot),
            BarKey::Unbound => self.ui.player_bar.keys.insert(slot, None),
            BarKey::Custom(gesture) => self.ui.player_bar.keys.insert(slot, Some(gesture)),
        };
    }

    fn load(profile: &Profile) -> UserSettings {
        let mut settings: UserSettings = profile
            .settings()
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        settings.ui.snap = settings.ui.snap.filter(|snap| snap.0 > 0.0);
        settings.ui.widgets.retain(|name, _| hud::keeps_place(name));
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
    menu: MenuSettings,
    #[serde(default)]
    player_bar: PlayerBarSettings,
    #[serde(default)]
    chat: ChatSettings,
    #[serde(default)]
    minimap: MinimapSettings,
    #[serde(default)]
    tracker_backing: Percent,
}

impl Default for UiSettings {
    fn default() -> UiSettings {
        UiSettings {
            snap: default_snap(),
            widgets: BTreeMap::new(),
            menu: MenuSettings::default(),
            player_bar: PlayerBarSettings::default(),
            chat: ChatSettings::default(),
            minimap: MinimapSettings::default(),
            tracker_backing: Percent::default(),
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
struct MenuSettings {
    #[serde(default)]
    placement: Option<Placement>,
    /// None opens on the tab last used this session.
    #[serde(default)]
    opens_on: Option<MenuTabName>,
}

#[derive(Serialize, Deserialize, Default)]
struct MinimapSettings {
    #[serde(default)]
    turns_with_camera: bool,
    /// None keeps the reach the game's sight gives.
    #[serde(default)]
    reach: Option<MinimapReachStep>,
}

#[derive(Serialize, Deserialize, Default)]
struct ChatSettings {
    #[serde(default)]
    hidden: bool,
    #[serde(default)]
    placement: Option<Placement>,
    #[serde(default)]
    background: Option<Percent>,
    #[serde(default)]
    text_size: Option<WindowPx>,
}

const CHAT_BACKGROUND: Percent = Percent(35.0);
const CHAT_TEXT_SIZES: (WindowPx, WindowPx, WindowPx) =
    (WindowPx(15.0), WindowPx(12.0), WindowPx(22.0));

#[derive(Serialize, Deserialize, Default)]
struct PlayerBarSettings {
    #[serde(default)]
    shape: Option<BarShape>,
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
