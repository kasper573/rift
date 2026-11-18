use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use strum::{IntoStaticStr, VariantArray};
use ui::controls::{self, KeyHint};
use ui::egui::{self, Pos2, Rect, Sense, Ui, UiBuilder, Vec2};
use ui::keys::KeyCap;
use ui::sheet::{self, Tab, TabSize};

use super::HudMessages;
use super::panel::{self, Sizing, WindowFrame, WindowKey};
use crate::core::content::{Content, Fixture};
use crate::core::gui;
use crate::systems::dialogue::stage;
use crate::systems::input::escape::{self, Dismissal};
use crate::systems::input::map::{self, InputAction, InputMap};
use crate::systems::input::scope;
use crate::systems::interface::{InterfaceIcon, InterfaceSound};
use crate::systems::messages;
use crate::systems::player::{self, session};
use crate::systems::screen::ClientScreen;
use crate::systems::settings::{self, Placed, UserSettings};
use crate::systems::{area_map, history, quest, skill, society};

pub trait MenuTab: Send + Sync {
    fn name(&self) -> MenuTabName;
    fn icon(&self) -> InterfaceIcon;
    fn opens(&self) -> InputAction;
    /// Whether the game has what the tab shows: a tab that is not shown has no place in the
    /// strip, and its key does nothing.
    fn shown(&self, _world: &World) -> bool {
        true
    }
    /// How much waits on the tab for the player, counted on its face.
    fn badge(&self, _world: &World) -> Option<u32> {
        None
    }
    fn sub_tabs(&self, _world: &World) -> Option<SubTabs> {
        None
    }
    fn choose_sub_tab(&self, _world: &mut World, _chosen: usize) {}
    /// The inputs that do something on the tab as it stands, and what each does.
    fn hints(&self, _world: &World) -> Vec<(InputAction, String)> {
        Vec::new()
    }
    fn show(&self, ui: &mut Ui, world: &mut World);
}

pub struct SubTabs {
    pub labels: Vec<String>,
    pub chosen: usize,
}

#[derive(
    Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, IntoStaticStr, VariantArray,
)]
#[serde(rename_all = "snake_case")]
pub enum MenuTabName {
    Character,
    Skills,
    Quests,
    Map,
    History,
    Social,
    Settings,
}

impl MenuTabName {
    pub fn named(self, words: &HudMessages) -> &str {
        match self {
            MenuTabName::Character => words.tab_character(),
            MenuTabName::Skills => words.tab_skills(),
            MenuTabName::Quests => words.tab_quests(),
            MenuTabName::Map => words.tab_map(),
            MenuTabName::History => words.tab_history(),
            MenuTabName::Social => words.tab_social(),
            MenuTabName::Settings => words.tab_settings(),
        }
    }
}

pub(crate) fn open(world: &mut World) {
    let menu = world.resource::<Menu>();
    let tab = world
        .resource::<UserSettings>()
        .menu_opens_on()
        .or(menu.last_open)
        .filter(|&tab| tab_of(tab).shown(world))
        .unwrap_or(TABS[0].name());
    open_on(world, tab);
}

pub(crate) fn open_on(world: &mut World, tab: MenuTabName) {
    if yields(world) || !tab_of(tab).shown(world) {
        return;
    }
    let mut menu = world.resource_mut::<Menu>();
    let was = menu.open.replace(tab);
    menu.last_open = Some(tab);
    if let Some(gui) = gui::context(world) {
        gui.move_to_top(panel::layer(WindowKey::Menu));
    }
    match was {
        None => InterfaceSound::UiOpen.play(world),
        Some(was) if was != tab => InterfaceSound::UiPage.play(world),
        Some(_) => {}
    }
}

pub(crate) fn toggle(world: &mut World, tab: MenuTabName) {
    if showing(world) == Some(tab) {
        close(world);
    } else {
        open_on(world, tab);
    }
}

pub(crate) fn close(world: &mut World) -> bool {
    let closed = world.resource_mut::<Menu>().open.take().is_some();
    if closed {
        InterfaceSound::UiClose.play(world);
    }
    closed
}

pub(crate) fn is_open(world: &World) -> bool {
    showing(world).is_some()
}

pub(crate) fn showing(world: &World) -> Option<MenuTabName> {
    world.resource::<Menu>().open
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<Menu>().add_systems(
        Update,
        (tab_keys, give_way)
            .chain()
            .run_if(in_state(ClientScreen::Area)),
    );
}

pub(super) fn draw(world: &mut World, gui: &egui::Context) {
    let Menu { open, last_open } = *world.resource::<Menu>();
    let Some(pose) = panel::sheet_pose(world, gui, WindowKey::Menu, open.is_some()) else {
        return;
    };
    let Some(shown) = open.or(last_open).filter(|&tab| tab_of(tab).shown(world)) else {
        return;
    };
    let screen = gui.content_rect();
    let frame = WindowFrame {
        key: WindowKey::Menu,
        of: Placed::Menu,
        title: None,
        start: Rect::from_min_size(
            (screen.center() - SIZE / 2.0).max(Pos2::new(super::MARGIN, super::MARGIN)),
            SIZE,
        ),
        sizing: Sizing::Resizable { min: SMALLEST },
    };
    let windowed = panel::window(world, gui, frame, pose, open.is_some(), |ui, world| {
        page(ui, world, frame, shown);
    });
    if windowed.closed {
        close(world);
    }
}

const SIZE: Vec2 = Vec2::new(960.0, 540.0);
const SMALLEST: Vec2 = Vec2::new(360.0, 240.0);
const SMALLEST_PAGE: Vec2 = Vec2::new(800.0, 330.0);
const BESIDE_HANDLE: f32 = 6.0;
const CURL_ROOM: f32 = 18.0;

// Their tabs line up in this order.
static TABS: &[&dyn MenuTab] = &[
    &player::sheet::CharacterMenuTab,
    &skill::window::SkillsMenuTab,
    &quest::log::QuestsMenuTab,
    &area_map::tab::MapMenuTab,
    &history::widget::HistoryMenuTab,
    &society::tab::SocialMenuTab,
    &settings::SettingsMenuTab,
];

#[derive(Resource, Default, Clone, Copy)]
struct Menu {
    open: Option<MenuTabName>,
    last_open: Option<MenuTabName>,
}

fn tab_keys(world: &mut World) {
    let pressed = TABS
        .iter()
        .find(|tab| tab.shown(world) && map::just_pressed(world, tab.opens()));
    if let Some(tab) = pressed {
        toggle(world, tab.name());
    }
}

fn give_way(world: &mut World) {
    if is_open(world) && yields(world) {
        close(world);
    }
}

/// A conversation takes the menu's place, and the fallen get up before anything else.
fn yields(world: &World) -> bool {
    stage::leavable(world) || session::is_dead(world)
}

fn tab_of(name: MenuTabName) -> &'static dyn MenuTab {
    TABS.iter()
        .copied()
        .find(|tab| tab.name() == name)
        .expect("every tab name has its tab")
}

fn page(ui: &mut Ui, world: &mut World, frame: WindowFrame, shown: MenuTabName) {
    let body = ui.max_rect();
    let pad = sheet::PANE_PAD;
    let cross_room = sheet::CROSS + 2.0 * sheet::CROSS_MARGIN;
    ui.add_space(STRIP_TOP);
    let mut chosen = shown;
    let strip_room = body.width() - pad.x - cross_room;
    let tabs = sheet::tabs_inset(
        ui,
        TabSize::Main,
        &strip(world),
        &mut chosen,
        strip_room,
        pad.x,
    );
    let bare = Rect::from_min_max(
        Pos2::new(tabs.max.x + BESIDE_HANDLE, body.min.y),
        Pos2::new(body.max.x - cross_room, tabs.max.y),
    );
    if bare.is_positive() {
        let content = world.resource::<Content>().clone();
        let words = &messages::of(&content).hud;
        panel::window_handle(ui, world, frame, words.menu(), bare);
    }
    if chosen != shown {
        open_on(world, chosen);
    }
    let tab = tab_of(shown);
    let mut top = tabs.max.y;
    if let Some(sub) = tab.sub_tabs(world) {
        let tabs: Vec<Tab<usize>> = sub
            .labels
            .iter()
            .enumerate()
            .map(|(index, label)| Tab::new(index, label.as_str()))
            .collect();
        let mut chosen = sub.chosen;
        let row = Rect::from_min_max(Pos2::new(body.min.x, top + SUB_STRIP_TOP), body.max);
        let strip = ui
            .scope_builder(UiBuilder::new().max_rect(row), |ui| {
                let room = row.width() - 2.0 * pad.x;
                super::tabs(ui, world, TabSize::Sub, &tabs, &mut chosen, room, pad.x)
            })
            .inner;
        top = strip.max.y;
        if chosen != sub.chosen {
            tab.choose_sub_tab(world, chosen);
        }
    }
    let room = Rect::from_min_max(Pos2::new(body.min.x, top), body.max);
    let foot_height = controls::HINT_LINE_HEIGHT + 2.0 * FOOT_PAD;
    let foot = Rect::from_min_max(Pos2::new(room.min.x, room.max.y - foot_height), room.max);
    let panes = Rect::from_min_max(room.min, Pos2::new(room.max.x, foot.min.y));
    ui.painter()
        .hline(body.x_range(), foot.min.y, sheet::rule_stroke());
    let squeezed = panes.width() < SMALLEST_PAGE.x || panes.height() < SMALLEST_PAGE.y;
    if squeezed {
        let area = egui::ScrollArea::both().id_salt(("menu page", <&str>::from(shown)));
        sheet::scroll_page(ui, panes, SMALLEST_PAGE, area, |ui| tab.show(ui, world));
    } else {
        ui.scope_builder(UiBuilder::new().max_rect(panes), |ui| {
            ui.shrink_clip_rect(panes);
            tab.show(ui, world);
        });
    }
    let hint_room = Rect::from_min_max(
        foot.min + Vec2::new(pad.x, FOOT_PAD),
        Pos2::new(
            (foot.max.x - pad.x).min(body.max.x - CURL_ROOM),
            foot.max.y - FOOT_PAD,
        ),
    );
    ui.scope_builder(UiBuilder::new().max_rect(hint_room), |ui| {
        hints(ui, world, tab);
    });
    ui.allocate_rect(room, Sense::hover());
}

const STRIP_TOP: f32 = 4.0;
const SUB_STRIP_TOP: f32 = 8.0;
const FOOT_PAD: f32 = 8.0;

fn strip(world: &mut World) -> Vec<Tab<MenuTabName>> {
    let content = world.resource::<Content>().clone();
    let words = &messages::of(&content).hud;
    let shown: Vec<&dyn MenuTab> = TABS
        .iter()
        .copied()
        .filter(|tab| tab.shown(world))
        .collect();
    shown
        .into_iter()
        .map(|tab| {
            let image = tab.icon().get(world.resource::<Content>()).image.clone();
            Tab {
                icon: gui::image(world, &image).map(|image| image.texture),
                keys: scope::reads(world, tab.opens())
                    .then(|| world.resource::<InputMap>().cap(tab.opens()))
                    .flatten(),
                badge: tab.badge(world).map(|waiting| badge_text(waiting, words)),
                ..Tab::new(tab.name(), tab.name().named(words))
            }
        })
        .collect()
}

/// The tabs the game has, in the strip's order.
pub(crate) fn tab_names(world: &World) -> Vec<MenuTabName> {
    TABS.iter()
        .filter(|tab| tab.shown(world))
        .map(|tab| tab.name())
        .collect()
}

fn badge_text(waiting: u32, words: &HudMessages) -> String {
    match waiting {
        0..=MOST_COUNTED => words.badge_count(waiting),
        _ => words.badge_over(MOST_COUNTED),
    }
}

const MOST_COUNTED: u32 = 99;

fn hints(ui: &mut Ui, world: &World, tab: &dyn MenuTab) {
    let keys = world.resource::<InputMap>();
    let words = &messages::of(world.resource::<Content>()).hud;
    let named: Vec<(KeyCap, String)> = tab
        .hints(world)
        .into_iter()
        .filter(|(action, _)| scope::reads(world, *action))
        .filter_map(|(action, does)| Some((keys.cap(action)?, does)))
        .collect();
    let hints: Vec<KeyHint> = named
        .iter()
        .map(|(keys, does)| KeyHint { keys, does })
        .collect();
    let leaves = (escape::dismisses(world) == Some(Dismissal::Sheet(WindowKey::Menu)))
        .then(|| keys.cap(InputAction::Dismiss))
        .flatten();
    let leaves = leaves.as_ref().map(|keys| KeyHint {
        keys,
        does: words.closes(),
    });
    controls::hint_line(ui, &hints, leaves);
}
