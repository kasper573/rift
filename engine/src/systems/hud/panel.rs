use bevy::prelude::*;
use egui::{Id, LayerId, Pos2, Rect, RichText, Sense, Stroke, Ui, UiBuilder, Vec2};

use super::{HudDrawn, WindowsDrawn};
use crate::core::gui::{self, Band};
use crate::core::math::{Pos, Size};
use crate::systems::settings::{Placement, UserSettings};

pub(crate) struct WindowFrame<'a> {
    pub key: &'static str,
    pub title: Option<&'a str>,
    pub start: Rect,
    pub sizing: Sizing,
    pub body_limit: Option<Vec2>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Sizing {
    Resizable,
    FitsContent,
}

pub(crate) fn widget<R>(
    world: &mut World,
    gui: &egui::Context,
    key: &'static str,
    start: Pos2,
    add: impl FnOnce(&mut Ui, &mut World) -> R,
) -> R {
    let id = panel_id(key);
    let saved = world
        .resource::<UserSettings>()
        .widget_pos(key)
        .map(|pos| Pos2::new(pos.x, pos.y));
    let rest = Rect::from_min_size(saved.unwrap_or(start), drawn_size(gui, id));
    let at = standing(world, gui, key, rest, None).min;
    let shown = egui::Area::new(id)
        .order(Band::Hud.order())
        .fixed_pos(at)
        .constrain(false)
        .sense(Sense::drag())
        .show(gui, |ui| add(ui, world));
    let rect = shown.response.rect;
    grab(
        world,
        &shown.response,
        Held::new(key, Kept::Spot, Grip::Move, rect, None),
    );
    world.resource_mut::<HudDrawn>().0.push(rect);
    world
        .resource_mut::<gui::WidgetRects>()
        .0
        .insert(key.to_owned(), rect);
    shown.inner
}

pub(crate) fn window<R>(
    world: &mut World,
    gui: &egui::Context,
    frame: WindowFrame,
    open: &mut bool,
    add: impl FnOnce(&mut Ui, &mut World) -> R,
) -> egui::InnerResponse<R> {
    let WindowFrame {
        key,
        title,
        start,
        sizing,
        body_limit,
    } = frame;
    let id = panel_id(key);
    let max = body_limit.map(|body| body + chrome(gui));
    let saved = world
        .resource::<UserSettings>()
        .window_placement(key)
        .map(placed_rect);
    let rest = match sizing {
        Sizing::Resizable => saved.unwrap_or(start),
        Sizing::FitsContent => Rect::from_min_size(
            saved.map_or(start.min, |saved| saved.min),
            Vec2::new(start.width(), drawn_size(gui, id).y),
        ),
    };
    let rect = standing(world, gui, key, rest, max);
    let shown = egui::Area::new(id)
        .order(Band::Hud.order())
        .fixed_pos(rect.min)
        .constrain(false)
        .show(gui, |ui| {
            let chrome = egui::Frame::window(ui.style());
            let margin = chrome.total_margin();
            let framed = chrome.show(ui, |ui| {
                ui.set_width(rect.width() - margin.sum().x);
                let bar = title_bar(ui, title, open);
                let inner = match sizing {
                    Sizing::Resizable => body(ui, rect.max.y - margin.bottom, |ui| add(ui, world)),
                    Sizing::FitsContent => fitted(ui, |ui| add(ui, world)),
                };
                (bar, inner)
            });
            let ((bar, inner), outer) = (framed.inner, framed.response.rect);
            let title_rect = Rect::from_min_max(outer.min, Pos2::new(outer.max.x, bar.max.y));
            let handle = ui.interact(title_rect, id.with("title"), Sense::drag());
            let grip = (sizing == Sizing::Resizable)
                .then(|| corner_grip(ui, id, outer, GripCorner::BottomRight));
            (handle, grip, inner)
        });
    let (handle, grip, inner) = shown.inner;
    let outer = shown.response.rect;
    grab(
        world,
        &handle,
        Held::new(key, Kept::Placement, Grip::Move, outer, max),
    );
    if let Some(grip) = grip {
        grab(
            world,
            &grip,
            Held::new(key, Kept::Placement, Grip::Resize, outer, max),
        );
    }
    world.resource_mut::<WindowsDrawn>().now.insert(key);
    world.resource_mut::<HudDrawn>().0.push(outer);
    egui::InnerResponse::new(inner, shown.response)
}

pub(super) fn shown_rect(gui: &egui::Context, key: &str) -> Option<Rect> {
    gui.memory(|memory| memory.area_rect(panel_id(key)))
}

pub(super) fn layer(key: &'static str) -> LayerId {
    LayerId::new(Band::Hud.order(), panel_id(key))
}

#[derive(Resource, Default)]
pub(super) struct Arranging(Option<Held>);

pub(super) fn arrange(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let Some(mut held) = world.resource::<Arranging>().0 else {
        return;
    };
    let (down, pointer) = gui.input(|input| (input.pointer.any_down(), input.pointer.latest_pos()));
    held.to = pointer.unwrap_or(held.to);
    if down {
        world.resource_mut::<Arranging>().0 = Some(held);
        return;
    }
    world.resource_mut::<Arranging>().0 = None;
    let screen = gui.content_rect();
    let settled = within(
        snapped(world, held.rect(screen), held.grip),
        screen,
        held.max,
    );
    let mut settings = world.resource_mut::<UserSettings>();
    match held.kept {
        Kept::Spot => settings.set_widget_pos(held.key, Pos::new(settled.min.x, settled.min.y)),
        Kept::Placement => settings.set_window_placement(
            held.key,
            Placement {
                pos: Pos::new(settled.min.x, settled.min.y),
                size: Size::new(settled.width(), settled.height()),
            },
        ),
    }
}

const MIN_WINDOW: Vec2 = Vec2::new(100.0, 100.0);

#[derive(Clone, Copy)]
pub(super) struct Held {
    key: &'static str,
    kept: Kept,
    grip: Grip,
    from: Rect,
    max: Option<Vec2>,
    grab: Pos2,
    to: Pos2,
}

impl Held {
    fn new(key: &'static str, kept: Kept, grip: Grip, from: Rect, max: Option<Vec2>) -> Held {
        Held {
            key,
            kept,
            grip,
            from,
            max,
            grab: Pos2::ZERO,
            to: Pos2::ZERO,
        }
    }

    // Measured from where it was grabbed rather than step by step, so wherever the screen's edge
    // holds the panel back, the spot grabbed is under the pointer again once the pointer returns.
    fn rect(&self, screen: Rect) -> Rect {
        let moved = self.to - self.grab;
        match self.grip {
            Grip::Move => self.from.translate(moved),
            Grip::Resize => Rect::from_min_size(
                self.from.min,
                (self.from.size() + moved)
                    .max(MIN_WINDOW)
                    .min(screen.max - self.from.min)
                    .min(largest(self.max)),
            ),
        }
    }
}

fn largest(max: Option<Vec2>) -> Vec2 {
    max.map_or(Vec2::INFINITY, |max| max.max(MIN_WINDOW))
}

#[derive(Clone, Copy)]
enum Kept {
    Spot,
    Placement,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Grip {
    Move,
    Resize,
}

fn grab(world: &mut World, handle: &egui::Response, held: Held) {
    if !handle.drag_started() {
        return;
    }
    let pointer = handle
        .ctx
        .input(|input| (input.pointer.press_origin(), input.pointer.latest_pos()));
    let (Some(grab), to) = pointer else {
        return;
    };
    world.resource_mut::<Arranging>().0 = Some(Held {
        grab,
        to: to.unwrap_or(grab),
        ..held
    });
}

fn standing(world: &World, gui: &egui::Context, key: &str, rest: Rect, max: Option<Vec2>) -> Rect {
    let screen = gui.content_rect();
    let rect = world
        .resource::<Arranging>()
        .0
        .filter(|held| held.key == key)
        .map_or(rest, |held| held.rect(screen));
    within(rect, screen, max)
}

fn within(rect: Rect, screen: Rect, max: Option<Vec2>) -> Rect {
    let size = rect.size().min(screen.size()).min(largest(max));
    Rect::from_min_size(rect.min.clamp(screen.min, screen.max - size), size)
}

/// What a window adds around its body: its frame and title bar.
pub(super) fn chrome(gui: &egui::Context) -> Vec2 {
    let style = gui.style_of(gui.theme());
    let margin = egui::Frame::window(&style).total_margin().sum();
    let title = gui.fonts_mut(|fonts| fonts.row_height(&egui::TextStyle::Heading.resolve(&style)));
    margin + Vec2::new(0.0, title + style.spacing.item_spacing.y)
}

fn snapped(world: &World, rect: Rect, grip: Grip) -> Rect {
    let Some(grid) = world.resource::<UserSettings>().snap() else {
        return rect;
    };
    let snap = |value: f32| (value / grid.0).round() * grid.0;
    match grip {
        Grip::Move => {
            Rect::from_min_size(Pos2::new(snap(rect.min.x), snap(rect.min.y)), rect.size())
        }
        Grip::Resize => Rect::from_min_size(
            rect.min,
            Vec2::new(snap(rect.width()), snap(rect.height())).max(MIN_WINDOW),
        ),
    }
}

fn title_bar(ui: &mut Ui, title: Option<&str>, open: &mut bool) -> Rect {
    let height = ui.text_style_height(&egui::TextStyle::Heading);
    let (_, bar) = ui.allocate_space(Vec2::new(ui.available_width(), height));
    if let Some(title) = title {
        ui.put(
            bar.shrink2(Vec2::new(height, 0.0)),
            egui::Label::new(RichText::new(title).heading())
                .selectable(false)
                .truncate(),
        );
    }
    let close = Rect::from_min_size(
        bar.right_top() - Vec2::new(height, 0.0),
        Vec2::splat(height),
    );
    if close_button(ui, close).clicked() {
        *open = false;
    }
    bar
}

fn close_button(ui: &mut Ui, rect: Rect) -> egui::Response {
    let response = ui.interact(rect, ui.id().with("close"), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "Close")
    });
    let visuals = ui.style().interact(&response);
    let cross = Rect::from_center_size(rect.center(), Vec2::splat(ui.spacing().icon_width - 4.0))
        .expand(visuals.expansion);
    let painter = ui.painter();
    painter.line_segment([cross.left_top(), cross.right_bottom()], visuals.fg_stroke);
    painter.line_segment([cross.right_top(), cross.left_bottom()], visuals.fg_stroke);
    response
}

fn body<R>(ui: &mut Ui, bottom: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let top = ui.cursor().min;
    let rect = Rect::from_min_max(
        top,
        Pos2::new(top.x + ui.available_width(), bottom.max(top.y)),
    );
    ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
        ui.set_clip_rect(rect.intersect(ui.clip_rect()));
        ui.set_min_size(rect.size());
        add(ui)
    })
    .inner
}

// An area lays its content out within the size it last had, which would hold a fitted window's
// scrolling content to its height for the item it showed before.
fn fitted<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let top = ui.cursor().min;
    let room = Rect::from_min_max(
        top,
        Pos2::new(
            top.x + ui.available_width(),
            ui.ctx().content_rect().bottom().max(top.y),
        ),
    );
    ui.scope_builder(UiBuilder::new().max_rect(room), add).inner
}

/// Which corner a resize grip sits in: the bottom right unless there is no room to pull there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GripCorner {
    BottomRight,
    TopRight,
}

pub(crate) fn corner_grip(ui: &mut Ui, id: Id, window: Rect, at: GripCorner) -> egui::Response {
    let size = ui.visuals().resize_corner_size;
    let (corner, cursor) = match at {
        GripCorner::BottomRight => (
            Rect::from_min_max(window.max - Vec2::splat(size), window.max),
            egui::CursorIcon::ResizeNwSe,
        ),
        GripCorner::TopRight => (
            Rect::from_min_max(
                Pos2::new(window.max.x - size, window.min.y),
                Pos2::new(window.max.x, window.min.y + size),
            ),
            egui::CursorIcon::ResizeNeSw,
        ),
    };
    let response = ui
        .interact(corner, id.with("grip"), Sense::drag())
        .on_hover_cursor(cursor);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::ResizeHandle, ui.is_enabled(), "Resize")
    });
    let stroke = Stroke::new(1.0, ui.style().interact(&response).fg_stroke.color);
    let inset = f32::from(ui.visuals().window_corner_radius.se) / 2.0;
    let (tip, down) = match at {
        GripCorner::BottomRight => (corner.max - Vec2::splat(inset), -1.0),
        GripCorner::TopRight => (corner.right_top() + Vec2::new(-inset, inset), 1.0),
    };
    let mut reach = 2.0;
    while reach <= size {
        ui.painter().line_segment(
            [
                tip - Vec2::new(reach, 0.0),
                tip + Vec2::new(0.0, down * reach),
            ],
            stroke,
        );
        reach += 4.0;
    }
    response
}

fn drawn_size(gui: &egui::Context, id: Id) -> Vec2 {
    gui.memory(|memory| memory.area_rect(id))
        .map_or(Vec2::ZERO, |rect| rect.size())
}

fn placed_rect(placement: Placement) -> Rect {
    Rect::from_min_size(
        Pos2::new(placement.pos.x, placement.pos.y),
        Vec2::new(placement.size.width, placement.size.height),
    )
}

fn panel_id(key: &str) -> Id {
    Id::new(("hud", key))
}
