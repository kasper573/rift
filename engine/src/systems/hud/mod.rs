mod panel;

use std::collections::HashSet;

use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use bevy_egui::input::egui_wants_any_keyboard_input;
use design_tokens::{HAIRLINE, SHADOW, TextRole, WINDOW_RADIUS, color, widgets};
use egui::{Id, LayerId, Pos2, Rect, Ui, Vec2};

use crate::core::assets::AssetRef;
use crate::core::content::{Content, Fixture};
use crate::core::gui::{self, Band};
use crate::core::math::WindowPx;
use crate::core::text::{self, StyledLine, TextLook};
use crate::systems::input::map::{ActionInput, InputAction, InputMap};
use crate::systems::interface::InterfaceIcon;
use crate::systems::screen::ClientScreen;
use crate::systems::settings;
use crate::systems::{equipment, history, item, player, quest, skill, stat, terminal};

pub(crate) use panel::{GripCorner, Sizing, WindowFrame, corner_grip, widget, window};

pub(crate) const WIDGET: WindowPx = WindowPx(48.0);
const WINDOW_SIZE: Vec2 = Vec2::new(400.0, 200.0);
const LAUNCHER_ICON: f32 = 32.0;
const MARGIN: f32 = 8.0;
const TOOLTIP_WIDTH: f32 = 260.0;
const CONFIRM: &str = "confirm";

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        settings::plugin(app);
        app.init_resource::<Open>()
            .init_resource::<panel::Arranging>()
            .init_resource::<WindowsDrawn>()
            .init_resource::<HudDrawn>()
            .init_resource::<Confirming>()
            .add_systems(
                Update,
                toggle_keys
                    .run_if(not(egui_wants_any_keyboard_input))
                    .run_if(in_state(ClientScreen::Area)),
            )
            .add_systems(
                EguiPrimaryContextPass,
                (
                    (forget_drawn, panel::arrange).before(Band::Stage),
                    draw_hud
                        .in_set(Band::Hud)
                        .run_if(in_state(ClientScreen::Area)),
                    draw_confirm.in_set(Band::Modal),
                ),
            );
    }
}

pub trait HudWidget: Send + Sync {
    fn fallback(&self, screen: Rect) -> Pos2;
    fn show(&self, ui: &mut Ui, world: &mut World);
}

pub trait HudWindow: Send + Sync {
    fn title(&self) -> &'static str;
    fn toggle(&self) -> InputAction;
    fn icon(&self) -> InterfaceIcon;
    fn size(&self) -> Vec2 {
        WINDOW_SIZE
    }
    /// The most body the window may be given, when its content has an edge it must never
    /// show past.
    fn body_limit(&self, _world: &World) -> Option<Vec2> {
        None
    }
    fn show(&self, ui: &mut Ui, world: &mut World);
}

pub(crate) struct Confirm {
    pub title: String,
    pub body: Vec<String>,
    pub confirm: String,
    pub cancel: String,
    pub on_confirm: Box<dyn FnOnce(&mut World) + Send + Sync>,
}

pub(crate) fn panel() -> egui::Frame {
    egui::Frame::new()
        .fill(color::PANEL)
        .stroke(HAIRLINE)
        .corner_radius(WINDOW_RADIUS)
        .shadow(SHADOW)
        .inner_margin(MARGIN)
}

pub(crate) fn icon(ui: &mut Ui, world: &mut World, asset: &AssetRef, size: f32) -> egui::Response {
    sized(ui, gui::image(world, asset), size)
}

pub(crate) fn confirm(world: &mut World, confirm: Confirm) {
    world.resource_mut::<Confirming>().0 = Some(Pending {
        confirm,
        focused: false,
    });
}

pub(crate) fn confirming(world: &World) -> bool {
    world.resource::<Confirming>().0.is_some()
}

pub(crate) fn dismiss_confirm(world: &mut World) -> bool {
    world.resource_mut::<Confirming>().0.take().is_some()
}

pub(crate) fn close_window(world: &mut World, window: &'static str) -> bool {
    world.resource_mut::<Open>().0.remove(&window)
}

pub(crate) fn window_open(world: &World, window: &'static str) -> bool {
    world.resource::<Open>().0.contains(&window)
}

pub(crate) fn just_opened(world: &World, window: &'static str) -> bool {
    !world.resource::<WindowsDrawn>().last.contains(window)
}

pub(crate) fn open_windows(world: &World) -> impl Iterator<Item = &'static str> + '_ {
    world.resource::<Open>().0.iter().copied()
}

pub(crate) fn window_layer(title: &'static str) -> LayerId {
    panel::layer(title)
}

pub(crate) fn confirm_layer() -> LayerId {
    LayerId::new(Band::Modal.order(), Id::new(CONFIRM))
}

pub(crate) fn pieces(world: &World) -> Vec<Rect> {
    world.resource::<HudDrawn>().0.clone()
}

pub(crate) fn tooltip(
    response: &egui::Response,
    world: &World,
    title: &str,
    lines: &[String],
    hint: Option<&StyledLine>,
) {
    gui::tooltip(response, |ui| {
        ui.set_max_width(TOOLTIP_WIDTH);
        ui.label(
            egui::RichText::new(title)
                .family(design_tokens::family::medium())
                .color(color::TEXT),
        );
        for line in lines {
            ui.label(
                egui::RichText::new(line)
                    .size(TextRole::Small.size())
                    .color(color::TEXT.gamma_multiply(0.85)),
            );
        }
        if let Some(hint) = hint {
            let look = TextLook {
                size: TextRole::Small.size(),
                color: color::MUTED,
            };
            text::show(ui, world, hint, look, None);
        }
    });
}

pub(crate) fn file_icon(ui: &mut Ui, world: &mut World, file: &str, size: f32) -> egui::Response {
    sized(ui, gui::image_file(world, file), size)
}

static WIDGETS: &[(&str, &dyn HudWidget)] = &[
    ("character", &player::widget::CharacterWidget),
    ("quests", &quest::tracker::QuestTrackerWidget),
    (skill::bar::HOTBAR, &skill::bar::HotbarWidget),
    (skill::bar::BASIC_ATTACK, &skill::bar::BasicAttackWidget),
];

// Their launchers line up in this order.
static WINDOWS: &[&dyn HudWindow] = &[
    &item::widget::InventoryWindow,
    &equipment::widget::EquipmentWindow,
    &stat::widget::StatsWindow,
    &skill::window::SkillsWindow,
    &quest::log::QuestLogWindow,
    &history::widget::HistoryWindow,
    &settings::SettingsWindow,
    &terminal::widget::TerminalWindow,
];

#[derive(Resource, Default)]
struct Open(HashSet<&'static str>);

#[derive(Resource, Default)]
struct WindowsDrawn {
    last: HashSet<&'static str>,
    now: HashSet<&'static str>,
}

#[derive(Resource, Default)]
struct HudDrawn(Vec<Rect>);

#[derive(Resource, Default)]
struct Confirming(Option<Pending>);

struct Pending {
    confirm: Confirm,
    focused: bool,
}

fn forget_drawn(mut drawn: ResMut<WindowsDrawn>, mut hud: ResMut<HudDrawn>) {
    let drawn = &mut *drawn;
    drawn.last = std::mem::take(&mut drawn.now);
    hud.0.clear();
}

/// Where a window stands this frame, while it is open.
pub(crate) fn window_rect(world: &World, window: &dyn HudWindow) -> Option<Rect> {
    let title = window.title();
    if !window_open(world, title) {
        return None;
    }
    panel::shown_rect(&gui::context(world)?, title)
}

/// The most a window may be given, when its content bounds it.
pub(crate) fn window_limit(world: &World, window: &dyn HudWindow) -> Option<Vec2> {
    let body = window.body_limit(world)?;
    Some(body + panel::chrome(&gui::context(world)?))
}

fn draw_hud(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let screen = gui.content_rect();
    for &(name, shown) in WIDGETS {
        widget(world, &gui, name, shown.fallback(screen), |ui, world| {
            shown.show(ui, world);
        });
    }
    for (slot, &shown) in WINDOWS.iter().enumerate() {
        let title = shown.title();
        if !window_open(world, title) {
            launcher(world, &gui, shown, launcher_pos(slot, screen));
            continue;
        }
        let size = shown.size();
        let centered = Rect::from_min_size(
            (screen.center() - size / 2.0).max(Pos2::new(MARGIN, MARGIN)),
            size,
        );
        let frame = WindowFrame {
            key: title,
            title: Some(title),
            start: centered,
            sizing: Sizing::Resizable,
            body_limit: shown.body_limit(world),
        };
        let mut open = true;
        window(world, &gui, frame, &mut open, |ui, world| {
            shown.show(ui, world)
        });
        if !open {
            close_window(world, title);
        }
    }
}

fn launcher(world: &mut World, gui: &egui::Context, opens: &'static dyn HudWindow, at: Pos2) {
    let title = opens.title();
    let image = opens.icon().get(world.resource::<Content>()).image.clone();
    let key = world
        .resource::<InputMap>()
        .name(opens.toggle())
        .unwrap_or_default();
    let button = widget(world, gui, title, at, |ui, world| {
        let response = ui.add(egui::Button::new("").min_size(Vec2::splat(WIDGET.0)));
        if let Some(icon) = gui::image(world, &image) {
            let at = Rect::from_center_size(response.rect.center(), Vec2::splat(LAUNCHER_ICON));
            ui.put(at, icon.widget(Vec2::splat(LAUNCHER_ICON)));
        }
        key_badge(ui, response.rect, &key);
        gui::tooltip(&response, |ui| {
            ui.label(title);
        });
        response
    });
    if button.clicked() {
        world.resource_mut::<Open>().0.insert(title);
    }
}

fn key_badge(ui: &Ui, button: Rect, key: &str) {
    let font = egui::FontId::monospace(10.0);
    let galley = ui
        .painter()
        .layout_no_wrap(key.to_owned(), font, color::MUTED);
    let size = galley.size() + Vec2::new(6.0, 2.0);
    let rect = Rect::from_min_size(button.right_bottom() - size - Vec2::splat(2.0), size);
    ui.painter()
        .rect(rect, 3.0, color::INPUT, HAIRLINE, egui::StrokeKind::Inside);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, color::MUTED);
}

fn draw_confirm(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let Some(mut pending) = world.resource_mut::<Confirming>().0.take() else {
        return;
    };
    let keep_key = world
        .resource::<InputMap>()
        .shortcut(InputAction::TakeDefault);
    let mut decided = None;
    let modal = egui::Modal::new(Id::new(CONFIRM)).show(&gui, |ui| {
        ui.set_max_width(360.0);
        ui.heading(&pending.confirm.title);
        ui.add_space(4.0);
        for line in &pending.confirm.body {
            ui.label(line);
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let keep = ui.button(&pending.confirm.cancel);
                if !pending.focused {
                    keep.request_focus();
                    pending.focused = true;
                }
                if widgets::danger(ui, &pending.confirm.confirm).clicked() {
                    decided = Some(true);
                }
                let keep_pressed =
                    keep_key.is_some_and(|key| ui.input_mut(|i| i.consume_shortcut(&key)));
                if keep.clicked() || keep_pressed {
                    decided = Some(false);
                }
            });
        });
    });
    if modal.should_close() && decided.is_none() {
        decided = Some(false);
    }
    match decided {
        Some(true) => (pending.confirm.on_confirm)(world),
        Some(false) => {}
        None => world.resource_mut::<Confirming>().0 = Some(pending),
    }
}

fn launcher_pos(slot: usize, screen: Rect) -> Pos2 {
    Pos2::new(
        screen.right() - MARGIN - WIDGET.0,
        screen.top() + MARGIN + slot as f32 * (WIDGET.0 + MARGIN),
    )
}

fn toggle_keys(input: ActionInput, mut open: ResMut<Open>) {
    for window in WINDOWS {
        let title = window.title();
        if input.just_pressed(window.toggle()) && !open.0.remove(title) {
            open.0.insert(title);
        }
    }
}

fn sized(ui: &mut Ui, image: Option<gui::GuiImage>, size: f32) -> egui::Response {
    match image {
        Some(image) => ui.add(image.widget(Vec2::splat(size))),
        None => {
            ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover())
                .1
        }
    }
}
