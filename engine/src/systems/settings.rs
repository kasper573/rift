use std::collections::{BTreeMap, BTreeSet};
use std::iter;
use ui::egui;
use ui::float::Hinted;
use ui::theme;

use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::VariantArray;
use ui::egui::Ui;
use ui::{controls, sheet};

use crate::core::audio::mix::{AudioCategory, AudioFader, AudioMix, AudioVolume};
use crate::core::content::{Content, Fixture};
use crate::core::gui::{self, GuiImage};
use crate::core::math::{Pos, Size};
use crate::core::os::Profile;
use crate::core::render::transition::{ScreenTransition, ScreenTransitionPreference};
use crate::core::render::video::{
    AmbientOcclusionQuality, AntiAliasingQuality, GrassQuality, PropDetail, Quality, ShadowQuality,
    VideoPreferences, ViewDistance,
};
use crate::core::render::{CameraPreferences, CameraRig};
use crate::core::text::{CharactersPerSecond, Typing};
use crate::core::units::{Percent, Seconds, WindowPx};
use crate::systems::actor::plate;
use crate::systems::area_map::{MapPicture, MapReachStep};
use crate::systems::attention::AttentionMark;
use crate::systems::chat::ChatEmbedKind;
use crate::systems::hud;
use crate::systems::hud::menu::{MenuTab, MenuTabName};
use crate::systems::input::map::{InputAction, KeyGesture};
use crate::systems::interface::InterfaceIcon;
use crate::systems::item::plates::ItemNaming;
use crate::systems::item::rarity::{ItemRarityDef, RarityRank};
use crate::systems::messages::{self, Messages, messages};
use crate::systems::player::session;
use crate::systems::skill::HotbarSlot;
use crate::systems::social_request::{self, setting::SocialChoice};
use crate::systems::society::SocietyView;

const SLIDER_WIDTH: f32 = 120.0;
const ROW_GAP: f32 = 8.0;
const SECTION_GAP: f32 = 8.0;
const VALUE_ROOM: f32 = 74.0;
const COLUMN: f32 =
    controls::FORM_LABELS + controls::FORM_SPACING.x + SLIDER_WIDTH + ROW_GAP + VALUE_ROOM;

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
    #[serde(default)]
    items: ItemSettings,
}

#[derive(Serialize, Deserialize, Clone, Copy)]
pub(crate) struct Placement {
    pub pos: Pos<WindowPx>,
    pub size: Size<WindowPx>,
}

/// A window the player places and the game keeps where they left it.
#[derive(
    Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(rename_all = "snake_case")]
pub enum Placed {
    Menu,
    Chat,
    Trade,
    Market,
    Roster,
    Shop,
    Crafting,
    Prompt,
    Entrance,
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
        let content = world.resource::<Content>().clone();
        let game = GameSide {
            messages: messages::of(&content),
            typing: world.get_resource::<Typing>().copied(),
            turn_ease: world
                .get_resource::<CameraRig>()
                .map(|rig| rig.framing().turn_ease),
            rarities: rarities(world.resource::<Content>()),
            tabs: hud::menu::tab_names(world),
            society: session::my_character(world)
                .and_then(|me| me.get::<SocietyView>())
                .cloned(),
            marks: [
                AttentionMark::QuestOffered,
                AttentionMark::News,
                AttentionMark::Talking,
            ]
            .map(|mark| gui::image(world, &mark.get(&content).icon))
            .to_vec(),
        };
        let ([], pane) = sheet::columns(ui, ui.available_rect_before_wrap(), []);
        let mut chosen = None;
        sheet::scroll_pane(ui, pane, egui::ScrollArea::vertical(), |ui| {
            chosen = show_settings(ui, &mut world.resource_mut::<UserSettings>(), &game);
        });
        if let Some(chosen) = chosen {
            chosen.send(world);
        }
    }
}

/// What the settings show that is the game's or the server's to say, not the device's.
struct GameSide<'a> {
    messages: &'a Messages,
    typing: Option<Typing>,
    turn_ease: Option<Seconds>,
    rarities: Vec<(RarityRank, String)>,
    /// The menu's tabs, as the game has them.
    tabs: Vec<MenuTabName>,
    /// How the player's account stands in society, once the server has said.
    society: Option<SocietyView>,
    /// The marks the sample plate wears.
    marks: Vec<Option<GuiImage>>,
}

/// The game's rarities by name, the lowest rank first.
fn rarities(content: &Content) -> Vec<(RarityRank, String)> {
    let mut rarities: Vec<(RarityRank, String)> = content
        .ids::<ItemRarityDef>()
        .map(|rarity| {
            let rarity = rarity.get(content);
            (rarity.rank, rarity.name.clone())
        })
        .collect();
    rarities.sort();
    rarities
}

fn show_settings(
    ui: &mut Ui,
    settings: &mut Mut<UserSettings>,
    game: &GameSide,
) -> Option<SocialChoice> {
    ui.spacing_mut().slider_width = SLIDER_WIDTH;
    if ui.available_width() < 2.0 * COLUMN + sheet::GUTTER {
        ui.spacing_mut().item_spacing.x = ROW_GAP;
        show_first_column(ui, settings, game);
        ui.add_space(SECTION_GAP);
        return show_second_column(ui, settings, game);
    }
    ui.spacing_mut().item_spacing.x = sheet::GUTTER;
    ui.columns(2, |columns| {
        let [left, right] = columns else {
            return None;
        };
        left.spacing_mut().item_spacing.x = ROW_GAP;
        right.spacing_mut().item_spacing.x = ROW_GAP;
        show_first_column(left, settings, game);
        show_second_column(right, settings, game)
    })
}

fn show_first_column(ui: &mut Ui, settings: &mut Mut<UserSettings>, game: &GameSide) {
    let words = &game.messages.settings;
    sheet::section(ui, words.section_interface());
    show_interface(ui, settings, game);
    ui.add_space(SECTION_GAP);
    sheet::section(ui, words.section_hud());
    show_hud(ui, settings, words);
    ui.add_space(SECTION_GAP);
    sheet::section(ui, words.section_items());
    show_items(ui, settings, &game.rarities, words);
    ui.add_space(SECTION_GAP);
    sheet::section(ui, words.section_camera());
    controls::form(ui, ("settings", "camera"), |ui| {
        if let Some(turn_ease) = game.turn_ease {
            let label = ui.label(words.camera_smoothing());
            smoothing_slider(ui, settings, turn_ease, words, label.id);
            ui.end_row();
        }
    });
}

fn show_second_column(
    ui: &mut Ui,
    settings: &mut Mut<UserSettings>,
    game: &GameSide,
) -> Option<SocialChoice> {
    let words = &game.messages.settings;
    sheet::section(ui, words.section_chat());
    show_chat(ui, settings, words);
    ui.add_space(SECTION_GAP);
    sheet::section(ui, words.section_video());
    show_video(ui, settings, words);
    ui.add_space(SECTION_GAP);
    sheet::section(ui, words.section_audio());
    controls::form(ui, ("settings", "audio"), |ui| {
        for fader in iter::once(AudioFader::Master).chain(
            AudioCategory::VARIANTS
                .iter()
                .copied()
                .map(AudioFader::Category),
        ) {
            let label = ui.label(fader_name(fader, words));
            fader_slider(ui, settings, words, fader, label.id);
            ui.end_row();
        }
    });
    let society = game.society.as_ref()?;
    ui.add_space(SECTION_GAP);
    sheet::section(ui, words.section_social());
    social_request::setting::section(ui, &game.messages.requests, society, true)
}

fn show_interface(ui: &mut Ui, settings: &mut Mut<UserSettings>, game: &GameSide) {
    let typing = game.typing;
    let words = &game.messages.settings;
    controls::form(ui, ("settings", "interface"), |ui| {
        let label = ui.label(words.menu_opens_on());
        menu_tab_picker(ui, settings, game, label.id);
        ui.end_row();
        let mut snapping = settings.snapping_enabled();
        if controls::toggle(ui, words.snapping(), &mut snapping).changed() {
            settings.toggle_snapping();
        }
        ui.end_row();
        let mut frame_rate = settings.frame_rate_shown();
        if controls::toggle(ui, words.show_fps(), &mut frame_rate).changed() {
            settings.toggle_frame_rate();
        }
        ui.end_row();
        let mut day_clock = settings.day_clock_shown();
        if controls::toggle(ui, words.show_time_of_day(), &mut day_clock).changed() {
            settings.toggle_day_clock();
        }
        ui.end_row();
        let mut amounts = settings.globe_amounts_shown();
        if controls::toggle(ui, words.globe_amounts(), &mut amounts)
            .hinted(words.globe_amounts_hint())
            .changed()
        {
            settings.toggle_globe_amounts();
        }
        ui.end_row();
        let mut quick_cast = settings.quick_cast();
        if controls::toggle(ui, words.quick_cast(), &mut quick_cast)
            .hinted(words.quick_cast_hint())
            .changed()
        {
            settings.toggle_quick_cast();
        }
        ui.end_row();
        let mut confirms = settings.confirms_risky_crafts();
        if controls::toggle(ui, words.confirm_crafts(), &mut confirms)
            .hinted(words.confirm_crafts_hint())
            .changed()
        {
            settings.toggle_risky_craft_confirm();
        }
        ui.end_row();
        let mut tilts = settings.cards_tilt();
        if controls::toggle(ui, words.tilt_cards(), &mut tilts)
            .hinted(words.tilt_cards_hint())
            .changed()
        {
            settings.toggle_card_tilt();
        }
        ui.end_row();
        if let Some(typing) = typing {
            let label = ui.label(words.text_speed());
            text_speed_slider(ui, settings, typing, words, label.id);
            ui.end_row();
        }
        let label = ui.label(words.nameplate_size());
        nameplate_slider(ui, settings, words, label.id);
        ui.end_row();
        ui.label("");
        nameplate_sample(ui, settings, game);
        ui.end_row();
        let label = ui.label(words.area_transition());
        let current = settings.screen_transition();
        if let Some(transition) = picker(
            ui,
            "area transition",
            current,
            |transition| transition_name(transition, words),
            label.id,
        ) {
            settings.set_screen_transition(transition);
        }
        ui.end_row();
    });
}

fn show_hud(ui: &mut Ui, settings: &mut Mut<UserSettings>, words: &SettingsMessages) {
    controls::form(ui, ("settings", "hud"), |ui| {
        let mut turns = settings.minimap_turns();
        if controls::toggle(ui, words.minimap_turns(), &mut turns)
            .hinted(words.minimap_turns_hint())
            .changed()
        {
            settings.toggle_minimap_turning();
        }
        ui.end_row();
        let label = ui.label(words.tracker_backing());
        tracker_backing_slider(ui, settings, words, label.id);
        ui.end_row();
        let label = ui.label(words.hud_places());
        if controls::button(ui, words.reset())
            .labelled_by(label.id)
            .hinted(words.reset_hint())
            .clicked()
        {
            settings.reset_hud();
        }
        ui.end_row();
    });
}

fn show_items(
    ui: &mut Ui,
    settings: &mut Mut<UserSettings>,
    rarities: &[(RarityRank, String)],
    words: &SettingsMessages,
) {
    controls::form(ui, ("settings", "items"), |ui| {
        let naming = settings.item_naming();
        let mut switched = naming.switched;
        if controls::toggle(ui, words.item_names_stay(), &mut switched)
            .hinted(words.item_names_stay_hint())
            .changed()
        {
            settings.items.naming.switched = switched;
        }
        ui.end_row();
        let mut as_dropped = naming.as_dropped;
        if controls::toggle(ui, words.name_as_dropped(), &mut as_dropped)
            .hinted(words.name_as_dropped_hint())
            .changed()
        {
            settings.items.naming.as_dropped = as_dropped;
        }
        ui.end_row();
        let label = ui.label(words.name_items_from());
        let name_of = |from: RarityRank| {
            rarities
                .iter()
                .rev()
                .find(|(rank, _)| *rank <= from)
                .or(rarities.first())
                .map_or("", |(_, name)| name.as_str())
        };
        let mut from = naming.from;
        controls::combo("name items from")
            .selected_text(name_of(naming.from))
            .show_ui(ui, |ui| {
                for (rank, name) in rarities {
                    ui.selectable_value(&mut from, *rank, name);
                }
            })
            .response
            .labelled_by(label.id)
            .hinted(words.name_items_from_hint());
        if from != naming.from {
            settings.items.naming.from = from;
        }
        ui.end_row();
        let mut always = settings.outlines_items_always();
        if controls::toggle(ui, words.outline_items(), &mut always)
            .hinted(words.outline_items_hint())
            .changed()
        {
            settings.items.outline_always = always;
        }
        ui.end_row();
        let mut calm = settings.rarity_effects_calm();
        if controls::toggle(ui, words.calm_rarity(), &mut calm)
            .hinted(words.calm_rarity_hint())
            .changed()
        {
            settings.items.calm_effects = calm;
        }
        ui.end_row();
    });
}

fn show_chat(ui: &mut Ui, settings: &mut Mut<UserSettings>, words: &SettingsMessages) {
    controls::form(ui, ("settings", "chat"), |ui| {
        let mut chat = settings.chat_shown();
        if controls::toggle(ui, words.show_chat(), &mut chat)
            .hinted(words.show_chat_hint())
            .changed()
        {
            settings.toggle_chat();
        }
        ui.end_row();
        let label = ui.label(words.chat_background());
        chat_background_slider(ui, settings, words, label.id);
        ui.end_row();
        let label = ui.label(words.chat_text_size());
        chat_text_slider(ui, settings, words, label.id);
        ui.end_row();
        let label = ui.label(words.embedded_chat());
        ui.horizontal(|ui| {
            for &kind in ChatEmbedKind::VARIANTS {
                let mut shown = settings.shows_chat_embeds(kind);
                if controls::check(ui, &mut shown, embed_name(kind, words))
                    .labelled_by(label.id)
                    .hinted(words.embedded_chat_hint())
                    .changed()
                {
                    settings.show_chat_embeds(kind, shown);
                }
            }
        });
        ui.end_row();
    });
}

fn menu_tab_picker(
    ui: &mut Ui,
    settings: &mut Mut<UserSettings>,
    game: &GameSide,
    label: egui::Id,
) {
    let last_used = game.messages.settings.last_used_tab();
    let named = |tab: MenuTabName| tab.named(&game.messages.hud);
    let chosen = settings.menu_opens_on();
    let mut picked = chosen;
    controls::combo("menu opens on")
        .selected_text(chosen.map_or(last_used, named))
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut picked, None, last_used);
            ui.separator();
            for &tab in &game.tabs {
                ui.selectable_value(&mut picked, Some(tab), named(tab));
            }
        })
        .response
        .labelled_by(label);
    if picked != chosen {
        settings.ui.menu.opens_on = picked;
    }
}

fn show_video(ui: &mut Ui, settings: &mut Mut<UserSettings>, words: &SettingsMessages) {
    let mut video = settings.video;
    controls::form(ui, "video", |ui| {
        let label = ui.label(words.quality());
        let current = video.quality();
        let mut picked = current;
        controls::combo("quality")
            .selected_text(current.map_or(words.quality_custom(), |quality| {
                quality_name(quality, words)
            }))
            .show_ui(ui, |ui| {
                for &quality in Quality::VARIANTS {
                    ui.selectable_value(&mut picked, Some(quality), quality_name(quality, words));
                }
            })
            .response
            .labelled_by(label.id);
        if let Some(quality) = picked.filter(|&quality| Some(quality) != current) {
            video = video.at(quality);
        }
        ui.end_row();
        controls::toggle(ui, words.fullscreen(), &mut video.fullscreen);
        ui.end_row();
        controls::toggle(ui, words.vsync(), &mut video.vsync).hinted(words.vsync_hint());
        ui.end_row();
        controls::toggle(ui, words.bloom(), &mut video.bloom).hinted(words.bloom_hint());
        ui.end_row();
        controls::toggle(ui, words.light_shafts(), &mut video.light_shafts)
            .hinted(words.light_shafts_hint());
        ui.end_row();
        let label = ui.label(words.shadows());
        let name = |shadows| shadow_name(shadows, words);
        if let Some(shadows) = picker(ui, "shadows", video.shadows, name, label.id) {
            video.shadows = shadows;
        }
        ui.end_row();
        let label = ui.label(words.ambient_occlusion());
        let name = |occlusion| occlusion_name(occlusion, words);
        if let Some(occlusion) = picker(
            ui,
            "ambient occlusion",
            video.ambient_occlusion,
            name,
            label.id,
        ) {
            video.ambient_occlusion = occlusion;
        }
        ui.end_row();
        let label = ui.label(words.anti_aliasing());
        let name = |anti_aliasing| anti_aliasing_name(anti_aliasing, words);
        if let Some(anti_aliasing) =
            picker(ui, "anti-aliasing", video.anti_aliasing, name, label.id)
        {
            video.anti_aliasing = anti_aliasing;
        }
        ui.end_row();
        let label = ui.label(words.view_distance());
        let name = |distance| distance_name(distance, words);
        if let Some(distance) = picker(ui, "view distance", video.view_distance, name, label.id) {
            video.view_distance = distance;
        }
        ui.end_row();
        let label = ui.label(words.grass());
        let name = |grass| grass_name(grass, words);
        if let Some(grass) = picker(ui, "grass", video.grass, name, label.id) {
            video.grass = grass;
        }
        ui.end_row();
        let label = ui.label(words.detail());
        let name = |detail| detail_name(detail, words);
        if let Some(detail) = picker(ui, "detail", video.detail, name, label.id) {
            video.detail = detail;
        }
        ui.end_row();
        let label = ui.label(words.map_picture());
        let name = |picture| map_picture_name(picture, words);
        let current = settings.map_picture();
        if let Some(picture) = picker(ui, "map picture", current, name, label.id) {
            settings.ui.minimap.picture = picture;
        }
        ui.end_row();
    });
    if video != settings.video {
        settings.video = video;
    }
}

fn picker<'a, T: VariantArray + Copy + PartialEq>(
    ui: &mut Ui,
    id_salt: &str,
    current: T,
    name: impl Fn(T) -> &'a str,
    label: egui::Id,
) -> Option<T> {
    let mut picked = current;
    controls::combo(id_salt)
        .selected_text(name(current))
        .show_ui(ui, |ui| {
            for &variant in T::VARIANTS {
                ui.selectable_value(&mut picked, variant, name(variant));
            }
        })
        .response
        .labelled_by(label);
    (picked != current).then_some(picked)
}

fn text_speed_slider(
    ui: &mut Ui,
    settings: &mut Mut<UserSettings>,
    typing: Typing,
    words: &SettingsMessages,
    label: egui::Id,
) {
    let mut speed = typing
        .pace_for(settings.text.characters_per_second)
        .characters
        .0;
    ui.horizontal(|ui| {
        let range = typing.slowest.0..=typing.fastest.0;
        if controls::slider(ui, &mut speed, range)
            .labelled_by(label)
            .changed()
        {
            settings.text.characters_per_second = Some(CharactersPerSecond(speed.round()));
        }
        slider_says(ui, words.characters_per_second(speed.round()));
    });
}

fn smoothing_slider(
    ui: &mut Ui,
    settings: &mut Mut<UserSettings>,
    game: Seconds,
    words: &SettingsMessages,
    label: egui::Id,
) {
    let mut ease = smoothing(settings.camera.turn_ease, game).0;
    ui.horizontal(|ui| {
        if controls::slider(ui, &mut ease, SMOOTHINGS.0.0..=SMOOTHINGS.1.0)
            .labelled_by(label)
            .changed()
        {
            settings.camera.turn_ease = Some(Seconds((ease * 100.0).round() / 100.0));
        }
        let shown = if ease > 0.0 {
            words.seconds_of((ease * 100.0).round() / 100.0)
        } else {
            words.smoothing_off().to_owned()
        };
        slider_says(ui, shown);
    });
}

fn chat_background_slider(
    ui: &mut Ui,
    settings: &mut Mut<UserSettings>,
    words: &SettingsMessages,
    label: egui::Id,
) {
    let mut background = settings.chat_background().0;
    ui.horizontal(|ui| {
        if controls::slider(ui, &mut background, 0.0..=Percent::WHOLE.0)
            .labelled_by(label)
            .changed()
        {
            settings.ui.chat.background = Some(Percent(background.round()));
        }
        slider_says(ui, words.percent_of(background.round()));
    });
}

fn tracker_backing_slider(
    ui: &mut Ui,
    settings: &mut Mut<UserSettings>,
    words: &SettingsMessages,
    label: egui::Id,
) {
    let mut backing = settings.tracker_backing().0;
    ui.horizontal(|ui| {
        if controls::slider(ui, &mut backing, 0.0..=Percent::WHOLE.0)
            .labelled_by(label)
            .changed()
        {
            settings.ui.tracker_backing = Percent(backing.round());
        }
        let shown = if backing > 0.0 {
            words.percent_of(backing.round())
        } else {
            words.tracker_backing_none().to_owned()
        };
        slider_says(ui, shown);
    });
}

fn nameplate_slider(
    ui: &mut Ui,
    settings: &mut Mut<UserSettings>,
    words: &SettingsMessages,
    label: egui::Id,
) {
    let mut size = settings.nameplate_size().0;
    ui.horizontal(|ui| {
        let (_, least, most) = NAMEPLATE_SIZES;
        if controls::slider(ui, &mut size, least.0..=most.0)
            .labelled_by(label)
            .hinted(words.nameplate_size_hint())
            .changed()
        {
            settings.ui.nameplate_size = Some(Percent((size / 5.0).round() * 5.0));
        }
        slider_says(ui, words.percent_of(settings.nameplate_size().0));
    });
}

/// A townsperson's plate at the size picked, with a quest to give and two more marks after its
/// name, drawn as the game draws plates.
fn nameplate_sample(ui: &mut Ui, settings: &Mut<UserSettings>, game: &GameSide) {
    let scale = settings.nameplate_size().0 / 100.0;
    let label = plate::PlateLabel::npc(game.messages.settings.nameplate_sample(), false);
    let tallest = plate::size_of(ui.ctx(), &label, &game.marks, NAMEPLATE_SIZES.2.0 / 100.0);
    let (rect, _) = ui.allocate_exact_size(tallest, egui::Sense::hover());
    plate::draw_marked(ui, rect.center_bottom(), &label, &game.marks, scale);
}

fn chat_text_slider(
    ui: &mut Ui,
    settings: &mut Mut<UserSettings>,
    words: &SettingsMessages,
    label: egui::Id,
) {
    let mut size = settings.chat_text_size().0;
    ui.horizontal(|ui| {
        let (_, smallest, largest) = CHAT_TEXT_SIZES;
        if controls::slider(ui, &mut size, smallest.0..=largest.0)
            .labelled_by(label)
            .changed()
        {
            settings.ui.chat.text_size = Some(WindowPx(size.round()));
        }
        slider_says(ui, words.pixels_of(size.round()));
    });
}

fn fader_slider(
    ui: &mut Ui,
    settings: &mut Mut<UserSettings>,
    words: &SettingsMessages,
    fader: AudioFader,
    label: egui::Id,
) {
    let mut volume = settings.volume(fader).0;
    ui.horizontal(|ui| {
        let range = AudioVolume::SILENT.0..=AudioVolume::FULL.0;
        if controls::slider(ui, &mut volume, range)
            .labelled_by(label)
            .changed()
        {
            settings.set_volume(fader, AudioVolume((volume * 100.0).round() / 100.0));
        }
        slider_says(ui, words.percent_of((volume * 100.0).round()));
    });
}

/// In room of its own, so a row is as wide whatever the slider says.
fn slider_says(ui: &mut Ui, set_to: String) {
    let room = egui::Vec2::new(VALUE_ROOM, ui.spacing().interact_size.y);
    let row = egui::Layout::left_to_right(egui::Align::Center);
    ui.allocate_ui_with_layout(room, row, |ui| {
        ui.set_min_width(VALUE_ROOM);
        ui.label(theme::faint(set_to));
    });
}

fn fader_name(fader: AudioFader, words: &SettingsMessages) -> &str {
    match fader {
        AudioFader::Master => words.master(),
        AudioFader::Category(AudioCategory::Music) => words.audio_music(),
        AudioFader::Category(AudioCategory::Ambience) => words.audio_ambience(),
        AudioFader::Category(AudioCategory::Effects) => words.audio_effects(),
        AudioFader::Category(AudioCategory::Voice) => words.audio_voice(),
    }
}

fn transition_name(transition: ScreenTransition, words: &SettingsMessages) -> &str {
    match transition {
        ScreenTransition::Crumble => words.transition_crumble(),
        ScreenTransition::TileWave => words.transition_tile_wave(),
        ScreenTransition::Sweep => words.transition_sweep(),
        ScreenTransition::Iris => words.transition_iris(),
        ScreenTransition::Dither => words.transition_dither(),
        ScreenTransition::Fade => words.transition_fade(),
    }
}

fn embed_name(kind: ChatEmbedKind, words: &SettingsMessages) -> &str {
    match kind {
        ChatEmbedKind::Item => words.embed_items(),
        ChatEmbedKind::Quest => words.embed_quests(),
        ChatEmbedKind::Skill => words.embed_skills(),
        ChatEmbedKind::Location => words.embed_locations(),
    }
}

fn quality_name(quality: Quality, words: &SettingsMessages) -> &str {
    match quality {
        Quality::Lowest => words.level_lowest(),
        Quality::Low => words.level_low(),
        Quality::Medium => words.level_medium(),
        Quality::High => words.level_high(),
        Quality::Max => words.level_max(),
    }
}

fn shadow_name(shadows: ShadowQuality, words: &SettingsMessages) -> &str {
    match shadows {
        ShadowQuality::Off => words.level_off(),
        ShadowQuality::Low => words.level_low(),
        ShadowQuality::High => words.level_high(),
    }
}

fn occlusion_name(occlusion: AmbientOcclusionQuality, words: &SettingsMessages) -> &str {
    match occlusion {
        AmbientOcclusionQuality::Off => words.level_off(),
        AmbientOcclusionQuality::Low => words.level_low(),
        AmbientOcclusionQuality::Medium => words.level_medium(),
        AmbientOcclusionQuality::High => words.level_high(),
    }
}

fn anti_aliasing_name(anti_aliasing: AntiAliasingQuality, words: &SettingsMessages) -> &str {
    match anti_aliasing {
        AntiAliasingQuality::Off => words.level_off(),
        AntiAliasingQuality::Low => words.level_low(),
        AntiAliasingQuality::Medium => words.level_medium(),
        AntiAliasingQuality::High => words.level_high(),
        AntiAliasingQuality::Ultra => words.level_ultra(),
    }
}

fn distance_name(distance: ViewDistance, words: &SettingsMessages) -> &str {
    match distance {
        ViewDistance::Near => words.level_near(),
        ViewDistance::Middle => words.level_middle(),
        ViewDistance::Far => words.level_far(),
    }
}

fn grass_name(grass: GrassQuality, words: &SettingsMessages) -> &str {
    match grass {
        GrassQuality::Low => words.level_low(),
        GrassQuality::Medium => words.level_medium(),
        GrassQuality::High => words.level_high(),
    }
}

fn map_picture_name(picture: MapPicture, words: &SettingsMessages) -> &str {
    match picture {
        MapPicture::Satellite => words.map_satellite(),
        MapPicture::Painted => words.map_painted(),
    }
}

fn detail_name(detail: PropDetail, words: &SettingsMessages) -> &str {
    match detail {
        PropDetail::Low => words.level_low(),
        PropDetail::Medium => words.level_medium(),
        PropDetail::High => words.level_high(),
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

    pub(crate) fn day_clock_shown(&self) -> bool {
        !self.screen.day_clock_hidden
    }

    pub(crate) fn toggle_day_clock(&mut self) {
        self.screen.day_clock_hidden = !self.screen.day_clock_hidden;
    }

    pub(crate) fn globe_amounts_shown(&self) -> bool {
        self.screen.globe_amounts
    }

    pub(crate) fn toggle_globe_amounts(&mut self) {
        self.screen.globe_amounts = !self.screen.globe_amounts;
    }

    pub(crate) fn item_naming(&self) -> ItemNaming {
        self.items.naming
    }

    pub(crate) fn item_names_on(&self) -> bool {
        self.items.names_on
    }

    pub(crate) fn switch_item_names(&mut self) {
        self.items.names_on = !self.items.names_on;
    }

    pub(crate) fn outlines_items_always(&self) -> bool {
        self.items.outline_always
    }

    pub(crate) fn rarity_effects_calm(&self) -> bool {
        self.items.calm_effects
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
        self.ui.placements.retain(|of, _| *of == Placed::Menu);
        self.ui.player_bar.shape = None;
    }

    pub(crate) fn placement(&self, of: Placed) -> Option<Placement> {
        self.ui.placements.get(&of).copied()
    }

    pub(crate) fn set_placement(&mut self, of: Placed, placement: Placement) {
        self.ui.placements.insert(of, placement);
    }

    pub(crate) fn menu_opens_on(&self) -> Option<MenuTabName> {
        self.ui.menu.opens_on
    }

    pub(crate) fn cards_tilt(&self) -> bool {
        !self.ui.flat_cards
    }

    pub(crate) fn toggle_card_tilt(&mut self) {
        self.ui.flat_cards = !self.ui.flat_cards;
    }

    pub(crate) fn minimap_turns(&self) -> bool {
        self.ui.minimap.turns_with_camera
    }

    pub(crate) fn toggle_minimap_turning(&mut self) {
        self.ui.minimap.turns_with_camera = !self.ui.minimap.turns_with_camera;
    }

    pub(crate) fn map_picture(&self) -> MapPicture {
        self.ui.minimap.picture
    }

    pub(crate) fn minimap_reach(&self) -> Option<MapReachStep> {
        self.ui.minimap.reach
    }

    pub(crate) fn set_minimap_reach(&mut self, step: MapReachStep) {
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

    /// Whether chat shows the things of `kind` its lines hold, rather than covering them.
    pub(crate) fn shows_chat_embeds(&self, kind: ChatEmbedKind) -> bool {
        !self.ui.chat.embeds_hidden.contains(&kind)
    }

    fn show_chat_embeds(&mut self, kind: ChatEmbedKind, shown: bool) {
        if shown {
            self.ui.chat.embeds_hidden.remove(&kind);
        } else {
            self.ui.chat.embeds_hidden.insert(kind);
        }
    }

    pub(crate) fn chat_background(&self) -> Percent {
        self.ui.chat.background.unwrap_or(CHAT_BACKGROUND)
    }

    /// How large plates are drawn, beside the size the game draws them.
    pub(crate) fn nameplate_size(&self) -> Percent {
        let (normal, least, most) = NAMEPLATE_SIZES;
        let size = self.ui.nameplate_size.unwrap_or(normal);
        Percent(size.0.clamp(least.0, most.0))
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
    placements: BTreeMap<Placed, Placement>,
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
    /// None keeps plates at the size the game draws them.
    #[serde(default)]
    nameplate_size: Option<Percent>,
    /// Whether cards lie flat rather than turn toward the pointer.
    #[serde(default)]
    flat_cards: bool,
}

impl Default for UiSettings {
    fn default() -> UiSettings {
        UiSettings {
            snap: default_snap(),
            widgets: BTreeMap::new(),
            placements: BTreeMap::new(),
            menu: MenuSettings::default(),
            player_bar: PlayerBarSettings::default(),
            chat: ChatSettings::default(),
            minimap: MinimapSettings::default(),
            tracker_backing: Percent::default(),
            nameplate_size: None,
            flat_cards: false,
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
struct MenuSettings {
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
    reach: Option<MapReachStep>,
    #[serde(default)]
    picture: MapPicture,
}

#[derive(Serialize, Deserialize, Default)]
struct ChatSettings {
    #[serde(default)]
    hidden: bool,
    #[serde(default)]
    background: Option<Percent>,
    #[serde(default)]
    text_size: Option<WindowPx>,
    /// The kinds of thing a line can hold that chat covers until the player asks to see one.
    #[serde(default)]
    embeds_hidden: BTreeSet<ChatEmbedKind>,
}

const CHAT_BACKGROUND: Percent = Percent(0.0);
/// How large plates are drawn: as the game draws them, and the least and most a player picks.
const NAMEPLATE_SIZES: (Percent, Percent, Percent) =
    (Percent(100.0), Percent(75.0), Percent(150.0));
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
    #[serde(default, alias = "letters_per_second")]
    characters_per_second: Option<CharactersPerSecond>,
}

#[derive(Serialize, Deserialize, Default)]
struct CraftingSettings {
    /// Whether a craft that may fail goes ahead unasked.
    #[serde(default)]
    unconfirmed: bool,
}

#[derive(Serialize, Deserialize, Default)]
struct ItemSettings {
    #[serde(default)]
    naming: ItemNaming,
    /// Whether names are switched on, for a player whose key switches them.
    #[serde(default)]
    names_on: bool,
    /// Whether the player's items are outlined as they lie, not only while named.
    #[serde(default)]
    outline_always: bool,
    /// Whether what a rarity plays on its items is held still rather than played.
    #[serde(default)]
    calm_effects: bool,
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
    #[serde(default, deserialize_with = "still_offered")]
    transition: ScreenTransition,
    #[serde(default)]
    frame_rate: bool,
    #[serde(default)]
    day_clock_hidden: bool,
    #[serde(default)]
    globe_amounts: bool,
}

/// A saved choice among what the interface offers, which is the default again once the
/// interface no longer offers it.
#[derive(Deserialize)]
#[serde(untagged)]
enum SavedChoice<T> {
    Offered(T),
    Gone(serde::de::IgnoredAny),
}

fn still_offered<'de, T, D>(deserializer: D) -> Result<T, D::Error>
where
    T: Deserialize<'de> + Default,
    D: serde::Deserializer<'de>,
{
    Ok(match SavedChoice::deserialize(deserializer)? {
        SavedChoice::Offered(choice) => choice,
        SavedChoice::Gone(_) => T::default(),
    })
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
        commands.insert_resource(typing.pace_for(settings.text.characters_per_second));
    }
}

fn apply_video(settings: Res<UserSettings>, mut video: ResMut<VideoPreferences>) {
    if settings.is_changed() {
        video.set_if_neq(settings.video);
    }
}

messages! {
    /// What the game says on its settings page.
    pub struct SettingsMessages {
        /// The heading of the settings page's interface section.
        section_interface,
        /// The heading of the settings page's hud section.
        section_hud,
        /// The heading of the settings page's items section.
        section_items,
        /// The heading of the settings page's camera section.
        section_camera,
        /// The heading of the settings page's chat section.
        section_chat,
        /// The heading of the settings page's video section.
        section_video,
        /// The heading of the settings page's audio section.
        section_audio,
        /// The heading of the settings page's social section.
        section_social,
        /// The label of the settings row for the tab the menu opens on.
        menu_opens_on,
        /// What the choice of the tab the menu opens on says for the tab last used.
        last_used_tab,
        /// The label of the settings row for snapping windows to a grid.
        snapping,
        /// The label of the settings row for showing the frame rate.
        show_fps,
        /// The label of the settings row for showing the day clock.
        show_time_of_day,
        /// The label of the settings row for writing health and mana in numbers.
        globe_amounts,
        /// What the tooltip of the settings row for writing health and mana in numbers says.
        globe_amounts_hint,
        /// The label of the settings row for casting skills at once.
        quick_cast,
        /// What the tooltip of the settings row for casting skills at once says.
        quick_cast_hint,
        /// The label of the settings row for asking before a craft that may fail.
        confirm_crafts,
        /// What the tooltip of the settings row for asking before a craft that may fail says.
        confirm_crafts_hint,
        /// The label of the settings row for cards turning toward the pointer.
        tilt_cards,
        /// What the tooltip of the settings row for cards turning toward the pointer says.
        tilt_cards_hint,
        /// What a slider says of a whole-number percentage, `{percent}` being the value.
        percent_of(percent: Number),
        /// What a slider says of a size in pixels, `{size}` being the value.
        pixels_of(size: Number),
        /// What a slider says of a number of seconds, `{seconds}` being the value.
        seconds_of(seconds: Number),
        /// The label of the settings row for how fast lines type.
        text_speed,
        /// What the text speed slider says of its value: the speed `{speed}`.
        characters_per_second(speed: Number),
        /// The label of the settings row for the transition between areas.
        area_transition,
        /// The name of the area transition `crumble` among the choices.
        transition_crumble,
        /// The name of the area transition `tile_wave` among the choices.
        transition_tile_wave,
        /// The name of the area transition `sweep` among the choices.
        transition_sweep,
        /// The name of the area transition `iris` among the choices.
        transition_iris,
        /// The name of the area transition `dither` among the choices.
        transition_dither,
        /// The name of the area transition `fade` among the choices.
        transition_fade,
        /// The label of the settings row for the minimap turning with the camera.
        minimap_turns,
        /// What the tooltip of the settings row for the minimap turning with the camera says.
        minimap_turns_hint,
        /// The label of the settings row for how dark the quest tracker's backing is.
        tracker_backing,
        /// What the quest tracker backing slider says of its value when there is no backing.
        tracker_backing_none,
        /// The label of the settings row for putting the HUD's pieces back.
        hud_places,
        /// The button that puts the HUD's pieces back where they started.
        reset,
        /// What the tooltip of the button that puts the HUD's pieces back says.
        reset_hint,
        /// The label of the settings row for item names staying on.
        item_names_stay,
        /// What the tooltip of the settings row for item names staying on says.
        item_names_stay_hint,
        /// The label of the settings row for naming items as they drop.
        name_as_dropped,
        /// What the tooltip of the settings row for naming items as they drop says.
        name_as_dropped_hint,
        /// The label of the settings row for the lowest rarity that is named.
        name_items_from,
        /// What the tooltip of the settings row for the lowest rarity that is named says.
        name_items_from_hint,
        /// The label of the settings row for outlining items.
        outline_items,
        /// What the tooltip of the settings row for outlining items says.
        outline_items_hint,
        /// The label of the settings row for holding rarity effects still.
        calm_rarity,
        /// What the tooltip of the settings row for holding rarity effects still says.
        calm_rarity_hint,
        /// The label of the settings row for how smoothly the camera turns.
        camera_smoothing,
        /// What the camera smoothing slider says of its value when there is none.
        smoothing_off,
        /// The label of the settings row for showing chat.
        show_chat,
        /// What the tooltip of the settings row for showing chat says.
        show_chat_hint,
        /// The label of the settings row for how dark chat's background is.
        chat_background,
        /// The label of the settings row for how large chat's text is.
        chat_text_size,
        /// The label of the settings row for how large the plates over heads are drawn.
        nameplate_size,
        /// What the tooltip of the settings row for how large plates are drawn says.
        nameplate_size_hint,
        /// The name on the sample plate the settings draw at the size picked.
        nameplate_sample,
        /// The label of the settings row for showing what chat lines embed.
        embedded_chat,
        /// What the tooltip of the settings row for showing what chat lines embed says.
        embedded_chat_hint,
        /// The name of the items that chat lines embed, beside the checkbox that shows them.
        embed_items,
        /// The name of the quests that chat lines embed, beside the checkbox that shows them.
        embed_quests,
        /// The name of the skills that chat lines embed, beside the checkbox that shows them.
        embed_skills,
        /// The name of the locations that chat lines embed, beside the checkbox that shows them.
        embed_locations,
        /// The name of the volume slider that sets every volume at once.
        master,
        /// The name of the volume slider of the music.
        audio_music,
        /// The name of the volume slider of the ambience.
        audio_ambience,
        /// The name of the volume slider of the effects.
        audio_effects,
        /// The name of the volume slider of the voice.
        audio_voice,
        /// The label of the settings row for the video quality preset.
        quality,
        /// What the video quality preset picker says when the parts stand at no preset.
        quality_custom,
        /// The label of the settings row for filling the screen.
        fullscreen,
        /// The label of the settings row for waiting for the display's refresh.
        vsync,
        /// What the tooltip of the settings row for waiting for the display's refresh says.
        vsync_hint,
        /// The label of the settings row for bloom.
        bloom,
        /// What the tooltip of the settings row for bloom says.
        bloom_hint,
        /// The label of the settings row for light shafts.
        light_shafts,
        /// What the tooltip of the settings row for light shafts says.
        light_shafts_hint,
        /// The label of the settings row for the quality of shadows.
        shadows,
        /// The label of the settings row for the quality of ambient occlusion.
        ambient_occlusion,
        /// The label of the settings row for the quality of anti-aliasing.
        anti_aliasing,
        /// The label of the settings row for how far the world stands around the player.
        view_distance,
        /// The label of the settings row for the quality of grass.
        grass,
        /// The label of the settings row for how close props draw their models.
        detail,
        /// The label of the settings row for how the minimap and the map picture the ground.
        map_picture,
        /// The choice of that row that pictures the ground as a satellite sees it.
        map_satellite,
        /// The choice of that row that paints the ground in the colours of what it is made of,
        /// which draws nothing more than the map.
        map_painted,
        /// The name of the video setting level `off` among the choices of any video setting that has it.
        level_off,
        /// The name of the video setting level `lowest` among the choices of any video setting that has it.
        level_lowest,
        /// The name of the video setting level `low` among the choices of any video setting that has it.
        level_low,
        /// The name of the video setting level `medium` among the choices of any video setting that has it.
        level_medium,
        /// The name of the video setting level `high` among the choices of any video setting that has it.
        level_high,
        /// The name of the video setting level `ultra` among the choices of any video setting that has it.
        level_ultra,
        /// The name of the video setting level `max` among the choices of any video setting that has it.
        level_max,
        /// The name of the video setting level `near` among the choices of any video setting that has it.
        level_near,
        /// The name of the video setting level `middle` among the choices of any video setting that has it.
        level_middle,
        /// The name of the video setting level `far` among the choices of any video setting that has it.
        level_far,
    }
}
