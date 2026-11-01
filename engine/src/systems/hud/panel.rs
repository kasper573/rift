use std::collections::HashMap;
use ui::egui;

use bevy::prelude::*;
use ui::egui::{Color32, Id, LayerId, Order, Pos2, Rect, Sense, Ui, Vec2};
use ui::motion::{Clock, SheetPose};
use ui::panel;
use ui::sheet::{self, Depth, Sheet};

use super::WindowsDrawn;
use crate::core::gui::{self, Band, Tier};
use crate::core::math::{Pos, Size};
use crate::systems::settings::{Placement, UserSettings};

pub(crate) struct WindowFrame<'a> {
    pub key: &'static str,
    pub title: Option<&'a str>,
    pub start: Rect,
    pub body_limit: Option<Vec2>,
}

pub(crate) fn widget<R>(
    world: &mut World,
    gui: &egui::Context,
    key: &'static str,
    start: Pos2,
    add: impl FnOnce(&mut Ui, &mut World) -> R,
) -> R {
    let id = widget_id(key);
    let saved = world
        .resource::<UserSettings>()
        .widget_pos(key)
        .map(|pos| Pos2::new(pos.x, pos.y));
    let rest = Rect::from_min_size(saved.unwrap_or(start), drawn_size(gui, id));
    let at = standing(world, gui, (key, Kept::Spot), rest, None).min;
    let shown = egui::Area::new(id)
        .order(Order::Background)
        .fixed_pos(at)
        .constrain(false)
        .fade_in(false)
        .sense(Sense::drag())
        .show(gui, |ui| add(ui, world));
    gui::stack(gui, Tier::Mid, shown.response.layer_id);
    let rect = shown.response.rect;
    grab(
        world,
        &shown.response,
        Held::new(key, Kept::Spot, Grip::Move, rect, None),
    );
    world
        .resource_mut::<gui::WidgetRects>()
        .0
        .insert(key.to_owned(), rect);
    shown.inner
}

/// `None` once a closed window has lifted off entirely.
pub(crate) fn sheet_pose(
    world: &mut World,
    gui: &egui::Context,
    key: &'static str,
    open: bool,
) -> Option<SheetPose> {
    let seconds = motion_step(gui);
    let mut clocks = world.resource_mut::<SheetClocks>();
    let clock = clocks
        .0
        .entry(key)
        .or_insert(if open { Clock::OPEN } else { Clock::CLOSED });
    clock.run(open, seconds);
    (!clock.is_closed() || open).then(|| clock.pose())
}

fn motion_step(gui: &egui::Context) -> f32 {
    gui.input(|input| input.unstable_dt).min(MAX_STEP)
}

pub(crate) fn window<R>(
    world: &mut World,
    gui: &egui::Context,
    frame: WindowFrame,
    pose: SheetPose,
    open: &mut bool,
    add: impl FnOnce(&mut Ui, &mut World) -> R,
) -> egui::InnerResponse<R> {
    let WindowFrame {
        key,
        title,
        start,
        body_limit,
    } = frame;
    let id = panel_id(key);
    let max = body_limit.map(|body| body + sheet::chrome(true));
    let rest = world
        .resource::<UserSettings>()
        .window_placement(key)
        .map(placed_rect)
        .unwrap_or(start);
    let rect = standing(world, gui, (key, Kept::Placement), rest, max);
    let front = front_sheet(world, gui) == Some(key);
    let at_rest = pose == SheetPose::REST;
    let shadow = match (at_rest, front) {
        (true, false) => sheet::shadow(Depth::Behind),
        _ => pose.shadow,
    };
    let pin = title.map_or(rect.min, |title| sheet::title_origin(gui, rect, title));
    gui.set_transform_layer(layer(key), pose.transform(gui, rect, pin));
    let shown = egui::Area::new(id)
        .order(Band::Hud.order())
        .fixed_pos(rect.min)
        .constrain(false)
        .fade_in(false)
        .show(gui, |ui| {
            ui.multiply_opacity(pose.opacity);
            let (outer, _) = ui.allocate_exact_size(rect.size(), Sense::hover());
            let sheet = Sheet {
                title,
                closable: true,
                curl: true,
                depth: if front { Depth::Front } else { Depth::Behind },
            };
            let parts = sheet.paint_at(ui.painter(), outer, shadow, Color32::WHITE);
            let handle = ui.interact(parts.band, id.with("title"), Sense::drag());
            if let Some(title) = title {
                handle.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Label, true, title)
                });
            }
            if let Some(cross) = parts.cross
                && panel::close_button(ui, id, cross).clicked()
            {
                *open = false;
            }
            let grip = parts.curl.map(|curl| panel::corner_grip(ui, id, curl));
            let inner = panel::body(ui, parts.body, |ui| add(ui, world));
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
    world
        .resource_mut::<gui::WidgetRects>()
        .0
        .insert(key.to_owned(), outer);
    egui::InnerResponse::new(inner, shown.response)
}

pub(crate) fn placed_spot(
    world: &World,
    gui: &egui::Context,
    key: &'static str,
    start: Pos2,
) -> Rect {
    let saved = world
        .resource::<UserSettings>()
        .widget_pos(key)
        .map(|pos| Pos2::new(pos.x, pos.y));
    let rest = Rect::from_min_size(saved.unwrap_or(start), drawn_size(gui, panel_id(key)));
    standing(world, gui, (key, Kept::Spot), rest, None)
}

pub(crate) fn grab_spot(world: &mut World, key: &'static str, handle: &egui::Response, rect: Rect) {
    grab(
        world,
        handle,
        Held::new(key, Kept::Spot, Grip::Move, rect, None),
    );
}

pub(crate) fn front_sheet(world: &World, gui: &egui::Context) -> Option<&'static str> {
    let drawn = &world.resource::<WindowsDrawn>().last;
    gui.memory(|memory| {
        memory
            .layer_ids()
            .filter_map(|layer_id| drawn.iter().copied().find(|key| layer(key) == layer_id))
            .last()
    })
}

pub(crate) fn mark_drawn(world: &mut World, key: &'static str) {
    world.resource_mut::<WindowsDrawn>().now.insert(key);
}

pub(crate) fn shown_rect(gui: &egui::Context, key: &str) -> Option<Rect> {
    gui.memory(|memory| memory.area_rect(panel_id(key)))
}

pub(crate) fn layer(key: &str) -> LayerId {
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
/// The most a sheet's clock moves in one frame, so a stalled frame never skips the motion.
const MAX_STEP: f32 = 1.0 / 30.0;

#[derive(Resource, Default)]
pub(super) struct SheetClocks(HashMap<&'static str, Clock>);

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

#[derive(Clone, Copy, PartialEq, Eq)]
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

/// A window and the launcher that opens it share a key, so the hold names which of the two it has.
fn standing(
    world: &World,
    gui: &egui::Context,
    (key, kept): (&str, Kept),
    rest: Rect,
    max: Option<Vec2>,
) -> Rect {
    let screen = gui.content_rect();
    let rect = world
        .resource::<Arranging>()
        .0
        .filter(|held| held.key == key && held.kept == kept)
        .map_or(rest, |held| held.rect(screen));
    within(rect, screen, max)
}

fn within(rect: Rect, screen: Rect, max: Option<Vec2>) -> Rect {
    let size = rect.size().min(screen.size()).min(largest(max));
    Rect::from_min_size(rect.min.clamp(screen.min, screen.max - size), size)
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

pub(crate) fn panel_id(key: &str) -> Id {
    Id::new(("hud", key))
}

fn widget_id(key: &str) -> Id {
    Id::new(("hud widget", key))
}
