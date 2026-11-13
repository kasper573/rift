use std::collections::HashMap;
use ui::egui;

use bevy::prelude::*;
use ui::egui::{Color32, Id, LayerId, Order, Pos2, Rect, Sense, Ui, Vec2};
use ui::motion::{SheetClock, SheetPose};
use ui::panel::{self, GripCorner};
use ui::sheet::{self, Depth, Sheet};

use super::WindowsDrawn;
use crate::core::gui::{self, Band, Tier};
use crate::core::math::{Pos, Size};
use crate::systems::settings::{Placed, Placement, UserSettings};

#[derive(Clone, Copy)]
pub(crate) struct WindowFrame<'a> {
    pub key: &'static str,
    pub of: Placed,
    pub title: Option<&'a str>,
    pub start: Rect,
    pub min: Vec2,
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
    let at = standing(world, gui, key, rest, Limits::NONE).min;
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
        Held::new(key, Kept::Spot, Grip::Move, rect, Limits::NONE),
    );
    world
        .resource_mut::<gui::WidgetRects>()
        .0
        .insert(key.to_owned(), rect);
    shown.inner
}

pub(crate) struct SizedPiece {
    pub key: &'static str,
    pub of: Placed,
    pub start: Rect,
    pub min: Vec2,
    pub max: Vec2,
    pub corner: GripCorner,
}

impl SizedPiece {
    fn limits(&self) -> Limits {
        Limits {
            min: self.min,
            max: Some(self.max),
        }
    }
}

pub(crate) fn sized_rect(world: &World, gui: &egui::Context, piece: &SizedPiece) -> Rect {
    let rest = world
        .resource::<UserSettings>()
        .placement(piece.of)
        .map(placed_rect)
        .unwrap_or(piece.start);
    standing(world, gui, piece.key, rest, piece.limits())
}

pub(crate) fn grab_sized(
    world: &mut World,
    piece: &SizedPiece,
    rect: Rect,
    mover: &egui::Response,
    sizer: &egui::Response,
) {
    let kept = Kept::Placement(piece.of);
    let held = |grip| Held::new(piece.key, kept, grip, rect, piece.limits());
    grab(world, mover, held(Grip::Move));
    grab(world, sizer, held(Grip::Resize(piece.corner)));
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
    let clock = clocks.0.entry(key).or_insert(if open {
        SheetClock::OPEN
    } else {
        SheetClock::CLOSED
    });
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
        of,
        title,
        start,
        min,
    } = frame;
    let id = panel_id(key);
    let limits = Limits { min, max: None };
    let kept = Kept::Placement(of);
    let rest = world
        .resource::<UserSettings>()
        .placement(of)
        .map(placed_rect)
        .unwrap_or(start);
    let rect = standing(world, gui, key, rest, limits);
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
        Held::new(key, kept, Grip::Move, outer, limits),
    );
    if let Some(grip) = grip {
        let resize = Grip::Resize(GripCorner::BottomRight);
        grab(world, &grip, Held::new(key, kept, resize, outer, limits));
    }
    world.resource_mut::<WindowsDrawn>().now.insert(key);
    world
        .resource_mut::<gui::WidgetRects>()
        .0
        .insert(key.to_owned(), outer);
    egui::InnerResponse::new(inner, shown.response)
}

pub(crate) fn window_handle(
    ui: &mut Ui,
    world: &mut World,
    frame: WindowFrame,
    name: &str,
    rect: Rect,
) {
    let id = panel_id(frame.key);
    let handle = ui.interact(rect, id.with("handle"), Sense::drag());
    handle.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, name));
    let Some(outer) = ui.memory(|memory| memory.area_rect(id)) else {
        return;
    };
    let limits = Limits {
        min: frame.min,
        max: None,
    };
    let kept = Kept::Placement(frame.of);
    grab(
        world,
        &handle,
        Held::new(frame.key, kept, Grip::Move, outer, limits),
    );
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
    standing(world, gui, key, rest, Limits::NONE)
}

pub(crate) fn grab_spot(world: &mut World, key: &'static str, handle: &egui::Response, rect: Rect) {
    let held = Held::new(key, Kept::Spot, Grip::Move, rect, Limits::NONE);
    grab(world, handle, held);
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
        snapped(world, held.rect(screen), held.grip, held.limits.min),
        screen,
        held.limits,
    );
    let mut settings = world.resource_mut::<UserSettings>();
    match held.kept {
        Kept::Spot => settings.set_widget_pos(held.key, Pos::new(settled.min.x, settled.min.y)),
        Kept::Placement(of) => settings.set_placement(
            of,
            Placement {
                pos: Pos::new(settled.min.x, settled.min.y),
                size: Size::new(settled.width(), settled.height()),
            },
        ),
    }
}

/// The most a sheet's clock moves in one frame, so a stalled frame never skips the motion.
const MAX_STEP: f32 = 1.0 / 30.0;

#[derive(Resource, Default)]
pub(super) struct SheetClocks(HashMap<&'static str, SheetClock>);

#[derive(Clone, Copy)]
pub(super) struct Held {
    key: &'static str,
    kept: Kept,
    grip: Grip,
    from: Rect,
    limits: Limits,
    grab: Pos2,
    to: Pos2,
}

#[derive(Clone, Copy)]
struct Limits {
    min: Vec2,
    max: Option<Vec2>,
}

impl Limits {
    const NONE: Limits = Limits {
        min: Vec2::ZERO,
        max: None,
    };
}

impl Held {
    fn new(key: &'static str, kept: Kept, grip: Grip, from: Rect, limits: Limits) -> Held {
        Held {
            key,
            kept,
            grip,
            from,
            limits,
            grab: Pos2::ZERO,
            to: Pos2::ZERO,
        }
    }

    // Measured from where it was grabbed rather than step by step, so wherever the screen's edge
    // holds the panel back, the spot grabbed is under the pointer again once the pointer returns.
    fn rect(&self, screen: Rect) -> Rect {
        let moved = self.to - self.grab;
        let fitted = |size: Vec2, room: Vec2| {
            size.max(self.limits.min)
                .min(room)
                .min(largest(self.limits))
        };
        match self.grip {
            Grip::Move => self.from.translate(moved),
            Grip::Resize(GripCorner::BottomRight) => Rect::from_min_size(
                self.from.min,
                fitted(self.from.size() + moved, screen.max - self.from.min),
            ),
            Grip::Resize(GripCorner::TopRight) => {
                let room = Vec2::new(
                    screen.right() - self.from.left(),
                    self.from.bottom() - screen.top(),
                );
                let pulled = self.from.size() + Vec2::new(moved.x, -moved.y);
                anchored_below(self.from, fitted(pulled, room))
            }
        }
    }
}

fn anchored_below(from: Rect, size: Vec2) -> Rect {
    Rect::from_min_size(Pos2::new(from.left(), from.bottom() - size.y), size)
}

fn largest(limits: Limits) -> Vec2 {
    limits.max.map_or(Vec2::INFINITY, |max| max.max(limits.min))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kept {
    Spot,
    Placement(Placed),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Grip {
    Move,
    Resize(GripCorner),
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

fn standing(world: &World, gui: &egui::Context, key: &str, rest: Rect, limits: Limits) -> Rect {
    let screen = gui.content_rect();
    let rect = world
        .resource::<Arranging>()
        .0
        .filter(|held| held.key == key)
        .map_or(rest, |held| held.rect(screen));
    within(rect, screen, limits)
}

fn within(rect: Rect, screen: Rect, limits: Limits) -> Rect {
    let size = rect.size().min(screen.size()).min(largest(limits));
    Rect::from_min_size(rect.min.clamp(screen.min, screen.max - size), size)
}

fn snapped(world: &World, rect: Rect, grip: Grip, min: Vec2) -> Rect {
    let Some(grid) = world.resource::<UserSettings>().snap() else {
        return rect;
    };
    let snap = |value: f32| (value / grid.0).round() * grid.0;
    let size = Vec2::new(snap(rect.width()), snap(rect.height())).max(min);
    match grip {
        Grip::Move => {
            Rect::from_min_size(Pos2::new(snap(rect.min.x), snap(rect.min.y)), rect.size())
        }
        Grip::Resize(GripCorner::BottomRight) => Rect::from_min_size(rect.min, size),
        Grip::Resize(GripCorner::TopRight) => anchored_below(rect, size),
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
