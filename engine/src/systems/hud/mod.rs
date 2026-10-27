mod card;
mod compass;
mod panel;

pub(crate) use card::{
    Pinned, draw_pinned, forget_pinned, pin, pinned, pinned_rect, raise_pinned, unpin,
};
pub(crate) use panel::{WindowFrame, sheet_pose, widget, window};

use std::collections::HashSet;
use ui::drag;
use ui::egui;
use ui::float;

use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use bevy_egui::input::egui_wants_any_keyboard_input;
use ui::egui::{
    Align2, Color32, Id, LayerId, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, Vec2,
};
use ui::face::{self, TextRole};
use ui::{controls, ink, sheet, slip};

use crate::core::assets::ImageRef;
use crate::core::content::{Content, Fixture};
use crate::core::gui::{self, Band, ImageSource, WidgetRects};
use crate::core::math::WindowPx;
use crate::core::text::{self, StyledLine, TextLook};
use crate::systems::input::map::{ActionInput, InputAction, InputMap};
use crate::systems::interface::InterfaceIcon;
use crate::systems::item::{self, Inventory};
use crate::systems::player::{self, session};
use crate::systems::screen::ClientScreen;
use crate::systems::settings::{self, UserSettings};
use crate::systems::skill::{self, HotbarEntry, HotbarSlot, SkillId};
use crate::systems::{equipment, history, quest, stat, terminal};
use ui::dialog;

pub(crate) const WIDGET: WindowPx = WindowPx(46.0);
pub(crate) const TOOLTIP_WIDTH: f32 = 260.0;
const WINDOW_SIZE: Vec2 = Vec2::new(400.0, 230.0);
const LAUNCHER_ICON: f32 = 32.0;
const LAUNCHER_RIGHT: f32 = 10.0;
const LAUNCHER_TOP: f32 = 9.0;
const LAUNCHER_PITCH: f32 = 55.0;
const MARGIN: f32 = 8.0;
const CARRIED: f32 = 40.0;
const CONFIRM: &str = "confirm";
const NOTE: Vec2 = Vec2::new(400.0, 170.0);

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Open>()
            .init_resource::<panel::Arranging>()
            .init_resource::<panel::SheetClocks>()
            .init_resource::<WindowsDrawn>()
            .init_resource::<Confirming>()
            .init_resource::<Lifting>()
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
                    draw_carried
                        .in_set(Band::Pointer)
                        .run_if(in_state(ClientScreen::Area)),
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

/// A question put to the player, answered with `true` for the confirming button and `false` for the
/// cancelling one or a dismissal.
pub(crate) struct Confirm {
    pub title: String,
    pub body: Vec<String>,
    pub confirm: String,
    pub cancel: String,
    pub answer: Answer,
}

pub(crate) type Answer = Box<dyn FnOnce(&mut World, bool) + Send + Sync>;

/// What the player drags across the interface, drawn under the pointer until it is dropped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Carried {
    Skill(SkillId),
    BagSlot(u32),
    Worn(equipment::EquipmentPlace),
    HotbarEntry {
        from: HotbarSlot,
        entry: HotbarEntry,
    },
}

pub fn icon(
    ui: &mut Ui,
    images: &mut (impl ImageSource + ?Sized),
    asset: &ImageRef,
    size: f32,
) -> egui::Response {
    sized(ui, images.image(asset), size)
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
    match world.resource_mut::<Confirming>().0.take() {
        Some(pending) => {
            (pending.confirm.answer)(world, false);
            true
        }
        None => false,
    }
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

pub(crate) fn launchers_width() -> f32 {
    LAUNCHER_RIGHT + WIDGET.0
}

/// Where every widget and window of the interface stood when last drawn.
pub(crate) fn pieces(world: &World) -> Vec<Rect> {
    world
        .resource::<WidgetRects>()
        .0
        .values()
        .copied()
        .collect()
}

pub(crate) fn tooltip(
    response: &egui::Response,
    world: &World,
    title: &str,
    lines: &[String],
    hint: Option<&StyledLine>,
) {
    float::tooltip(response, |ui| {
        ui.set_max_width(TOOLTIP_WIDTH);
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.label(face::bold(title).size(TextRole::Tooltip.size()));
        for line in lines {
            ui.label(
                egui::RichText::new(line)
                    .size(TextRole::Tooltip.size())
                    .color(ink::INK2),
            );
        }
        if let Some(hint) = hint {
            let look = TextLook::new(TextRole::Small.size(), ink::INK3);
            text::show(ui, world, hint, look, None);
        }
    });
}

pub(crate) fn file_icon(ui: &mut Ui, world: &mut World, file: &str, size: f32) -> egui::Response {
    sized(ui, gui::image_file(world, file), size)
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
    Some(body + sheet::chrome(true))
}

static WIDGETS: &[(&str, &dyn HudWidget)] = &[
    ("character", &player::widget::CharacterWidget),
    ("compass", &compass::CompassWidget),
    ("quests", &quest::tracker::QuestTrackerWidget),
    (skill::bar::HOTBAR, &skill::bar::HotbarWidget),
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
struct Confirming(Option<Pending>);

/// What the confirm note says, kept while the note lifts off after it is answered.
#[derive(Resource, Default)]
struct Lifting(Option<NoteText>);

#[derive(Clone)]
struct NoteText {
    title: String,
    body: Vec<String>,
    confirm: String,
    cancel: String,
}

impl NoteText {
    fn of(confirm: &Confirm) -> NoteText {
        NoteText {
            title: confirm.title.clone(),
            body: confirm.body.clone(),
            confirm: confirm.confirm.clone(),
            cancel: confirm.cancel.clone(),
        }
    }
}

struct Pending {
    confirm: Confirm,
    focused: bool,
}

fn forget_drawn(mut drawn: ResMut<WindowsDrawn>) {
    let drawn = &mut *drawn;
    drawn.last = std::mem::take(&mut drawn.now);
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
    let spots = launcher_spots(world, screen);
    for (&shown, spot) in WINDOWS.iter().zip(spots) {
        let title = shown.title();
        let is_open = window_open(world, title);
        launcher(world, &gui, shown, spot, is_open);
        let Some(pose) = panel::sheet_pose(world, &gui, title, is_open) else {
            continue;
        };
        let size = shown.size();
        let centered = Rect::from_min_size(
            (screen.center() - size / 2.0).max(Pos2::new(MARGIN, MARGIN)),
            size,
        );
        let frame = WindowFrame {
            key: title,
            title: Some(title),
            start: centered,
            body_limit: shown.body_limit(world),
        };
        let mut open = is_open;
        window(world, &gui, frame, pose, &mut open, |ui, world| {
            if !is_open {
                ui.disable();
            }
            shown.show(ui, world)
        });
        if is_open && !open {
            close_window(world, title);
        }
    }
}

fn launcher(
    world: &mut World,
    gui: &egui::Context,
    opens: &'static dyn HudWindow,
    at: Pos2,
    is_open: bool,
) {
    let title = opens.title();
    let image = opens.icon().get(world.resource::<Content>()).image.clone();
    let key = world
        .resource::<InputMap>()
        .name(opens.toggle())
        .unwrap_or_default();
    let button = widget(world, gui, title, at, |ui, world| {
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(WIDGET.0), Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, ""));
        let painter = ui.painter();
        painter.add(slip::SHADOW.as_shape(rect, 3.0));
        sheet::radial(painter, rect);
        let hovered = response.hovered();
        painter.rect(
            rect,
            3.0,
            if hovered {
                ink::gold_wash(0.2)
            } else {
                Color32::TRANSPARENT
            },
            Stroke::new(
                1.0,
                if hovered {
                    ink::OCHRE
                } else {
                    ink::ink_wash(0.55)
                },
            ),
            StrokeKind::Inside,
        );
        painter.line_segment(
            [
                rect.left_top() + Vec2::new(1.0, 1.5),
                rect.right_top() + Vec2::new(-1.0, 1.5),
            ],
            Stroke::new(1.0, ink::ink_wash(0.15)),
        );
        if let Some(icon) = gui::image(world, &image) {
            let at = Rect::from_center_size(rect.center(), Vec2::splat(LAUNCHER_ICON));
            ui.put(at, icon.widget(Vec2::splat(LAUNCHER_ICON)));
        }
        let painter = ui.painter();
        painter.text(
            rect.right_bottom() - Vec2::new(4.0, 2.0),
            Align2::RIGHT_BOTTOM,
            &key,
            TextRole::Launcher.font_id(),
            ink::INK,
        );
        if is_open {
            painter.rect_filled(rect, 3.0, ink::wash(ink::CREAM, 0.55));
        }
        if response.has_focus() {
            controls::focus_ring(painter, rect, 3.0);
        }
        float::tooltip(&response, |ui| {
            ui.label(face::bold(title).size(TextRole::Tooltip.size()));
        });
        response
    });
    if button.clicked() {
        world.resource_mut::<Open>().0.insert(title);
    }
}

fn draw_confirm(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let pending = world.resource_mut::<Confirming>().0.take();
    let shown_text = pending
        .as_ref()
        .map(|pending| NoteText::of(&pending.confirm));
    if let Some(text) = &shown_text {
        world.resource_mut::<Lifting>().0 = Some(text.clone());
    }
    let Some(pose) = panel::sheet_pose(world, &gui, CONFIRM, pending.is_some()) else {
        world.resource_mut::<Lifting>().0 = None;
        return;
    };
    let Some(text) = shown_text.or_else(|| world.resource::<Lifting>().0.clone()) else {
        return;
    };
    let keep_key = world
        .resource::<InputMap>()
        .shortcut(InputAction::TakeDefault);
    let mut pending = pending;
    let mut decided = None;
    let screen = gui.content_rect();
    let rest = Rect::from_center_size(screen.center(), NOTE);
    gui.set_transform_layer(confirm_layer(), pose.transform(&gui, rest, rest.min));
    let modal = egui::Modal::new(Id::new(CONFIRM))
        .backdrop_color(ink::ink_wash(0.3 * pose.opacity))
        .frame(egui::Frame::NONE)
        .show(&gui, |ui| {
            ui.multiply_opacity(pose.opacity);
            if pending.is_none() {
                ui.disable();
            }
            let under = ui.painter().add(Shape::Noop);
            let framed = egui::Frame::new()
                .inner_margin(dialog::PADDING)
                .show(ui, |ui| {
                    let padding = 2.0 * f32::from(dialog::PADDING);
                    ui.set_width(NOTE.x - padding);
                    ui.set_min_height(NOTE.y - padding);
                    dialog::title(ui, &text.title);
                    for line in &text.body {
                        ui.add(egui::Label::new(face::body(line)).wrap());
                    }
                    ui.add_space(12.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Max), |ui| {
                        let keep = controls::primary(ui, text.cancel.as_str());
                        if let Some(pending) = pending.as_mut()
                            && !pending.focused
                        {
                            keep.request_focus();
                            pending.focused = true;
                        }
                        if controls::danger(ui, text.confirm.as_str()).clicked() {
                            decided = Some(true);
                        }
                        let keep_pressed =
                            keep_key.is_some_and(|key| ui.input_mut(|i| i.consume_shortcut(&key)));
                        if keep.clicked() || keep_pressed {
                            decided = Some(false);
                        }
                    });
                });
            let note = framed.response.rect;
            let shapes = vec![
                Shape::Rect(pose.shadow.as_shape(note, 0.0)),
                sheet::vellum_shape(ui.ctx(), note),
            ];
            ui.painter().set(under, Shape::Vec(shapes));
            sheet::rule(ui.painter(), note, sheet::RULE_INSET);
        });
    let Some(pending) = pending else {
        return;
    };
    if modal.should_close() && decided.is_none() {
        decided = Some(false);
    }
    match decided {
        Some(decided) => (pending.confirm.answer)(world, decided),
        None => world.resource_mut::<Confirming>().0 = Some(pending),
    }
}

/// The icon of what is carried, under the pointer.
fn draw_carried(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let Some(carried) = drag::carried::<Carried>(&gui) else {
        return;
    };
    let content = world.resource::<Content>().clone();
    let icon = match carried {
        Carried::Skill(skill) => skill.get(&content).icon.clone(),
        Carried::BagSlot(slot) => {
            let Some(stack) = session::my_character(world)
                .and_then(|me| me.get::<Inventory>())
                .and_then(|inventory| inventory.slots.get(slot as usize).copied())
            else {
                return;
            };
            stack.item.get(&content).icon.clone()
        }
        Carried::Worn(slot) => {
            let Some(item) = session::me(world).and_then(|me| equipment::in_slot(world, me, &slot))
            else {
                return;
            };
            item.get(&content).icon.clone()
        }
        Carried::HotbarEntry { entry, .. } => match entry {
            HotbarEntry::Skill(skill) => skill.get(&content).icon.clone(),
            HotbarEntry::Item(item) => item.get(&content).icon.clone(),
            HotbarEntry::Empty => return,
        },
    };
    let Some(image) = gui::image(world, &icon) else {
        return;
    };
    float::at_pointer(
        &gui,
        Id::new("carried"),
        Band::Pointer.order(),
        -Vec2::splat(CARRIED / 2.0),
        |ui| ui.add(image.widget(Vec2::splat(CARRIED))),
    );
}

/// Where each launcher rests when the player never moved it: its own slot down the column, or
/// the next one no other launcher already stands on, since spots kept from an older column can
/// cover a launcher that was added since.
fn launcher_spots(world: &World, screen: Rect) -> Vec<Pos2> {
    let settings = world.resource::<UserSettings>();
    let saved = |window: &dyn HudWindow| {
        settings
            .widget_pos(window.title())
            .map(|pos| Pos2::new(pos.x, pos.y))
    };
    let mut taken: Vec<Pos2> = WINDOWS.iter().filter_map(|window| saved(*window)).collect();
    let covered = |at: Pos2, taken: &[Pos2]| {
        let rect = Rect::from_min_size(at, Vec2::splat(WIDGET.0));
        taken
            .iter()
            .any(|other| rect.intersects(Rect::from_min_size(*other, Vec2::splat(WIDGET.0))))
    };
    let mut spots = Vec::with_capacity(WINDOWS.len());
    for (slot, window) in WINDOWS.iter().enumerate() {
        let spot = saved(*window).unwrap_or_else(|| {
            let free = (slot..)
                .map(|slot| launcher_pos(slot, screen))
                .find(|at| !covered(*at, &taken))
                .expect("the column runs on");
            taken.push(free);
            free
        });
        spots.push(spot);
    }
    spots
}

fn launcher_pos(slot: usize, screen: Rect) -> Pos2 {
    Pos2::new(
        screen.right() - LAUNCHER_RIGHT - WIDGET.0,
        screen.top() + LAUNCHER_TOP + slot as f32 * LAUNCHER_PITCH,
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
        None => ui.allocate_exact_size(Vec2::splat(size), Sense::hover()).1,
    }
}
