use std::collections::HashMap;
use ui::egui;
use ui::float;

use bevy::prelude::*;
use ui::egui::{Align2, Color32, Id, LayerId, Order, Pos2, Rect, Sense, Ui, Vec2};
use ui::motion::{SheetClock, SheetPose};
use ui::panel::{self, GripCorner};
use ui::sheet::{self, Depth, Sheet, Tab, TabSize};

use super::{PlacedWindow, WindowsDrawn};
use crate::core::content::Content;
use crate::core::gui::{self, Band, Tier};
use crate::core::math::{Pos, Size};
use crate::systems::interface::InterfaceSound;
use crate::systems::messages;
use crate::systems::settings::{Placed, Placement, UserSettings};

/// A window's strip of tabs, `inset` in from the left of `room`: the one strip every window turns
/// its pages with, to the page sound as a tab is chosen.
pub(crate) fn tabs<T: PartialEq + Copy>(
    ui: &mut Ui,
    world: &mut World,
    size: TabSize,
    tabs: &[Tab<T>],
    active: &mut T,
    room: f32,
    inset: f32,
) -> Rect {
    let was = *active;
    let strip = sheet::tabs_inset(ui, size, tabs, active, room, inset);
    if *active != was {
        InterfaceSound::UiPage.play(world);
    }
    strip
}

/// Every window and card of the interface that stacks, keeps a place and lifts on and off as a
/// sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum WindowKey {
    Menu,
    Market,
    Trade,
    Roster,
    ItemCard,
    SkillCard,
    QuestCard,
    Confirm,
    Stage,
    Shop,
    Crafting,
    Prompt,
    Entrance,
    QuestOffer,
}

impl WindowKey {
    /// The windows and cards a conversation opens, which close with it.
    pub(crate) const OF_A_CONVERSATION: [WindowKey; 5] = [
        WindowKey::Shop,
        WindowKey::Crafting,
        WindowKey::Prompt,
        WindowKey::Entrance,
        WindowKey::QuestOffer,
    ];

    /// What names the window where something outside the code holds it: the place a card keeps in
    /// the player's settings, and the rect a script's effect can aim at.
    pub(crate) fn name(self) -> &'static str {
        match self {
            WindowKey::Menu => "menu",
            WindowKey::Market => "market",
            WindowKey::Trade => "trade_window",
            WindowKey::Roster => "instance_roster",
            WindowKey::ItemCard => "Item",
            WindowKey::SkillCard => "Skill",
            WindowKey::QuestCard => "Quest",
            WindowKey::Confirm => "confirm",
            WindowKey::Stage => "stage",
            WindowKey::Shop => "shop",
            WindowKey::Crafting => "crafting",
            WindowKey::Prompt => "prompt",
            WindowKey::Entrance => "entrance",
            WindowKey::QuestOffer => "quest_offer",
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct WindowFrame<'a> {
    pub key: WindowKey,
    pub of: Placed,
    pub title: Option<&'a str>,
    pub start: Rect,
    pub sizing: Sizing,
}

/// Whether the player sizes a sheet, or it keeps the size of its `start`.
#[derive(Clone, Copy)]
pub(crate) enum Sizing {
    Fixed,
    Resizable { min: Vec2 },
}

impl Sizing {
    fn limits(self, start: Rect) -> Limits {
        match self {
            Sizing::Fixed => Limits {
                min: start.size(),
                max: Some(start.size()),
            },
            Sizing::Resizable { min } => Limits { min, max: None },
        }
    }
}

pub(crate) fn widget<R>(
    world: &mut World,
    gui: &egui::Context,
    key: &'static str,
    (corner, start): (Align2, Pos2),
    add: impl FnOnce(&mut Ui, &mut World) -> R,
) -> R {
    let id = widget_id(key);
    let size = drawn_size(gui, id);
    let saved = world.resource::<UserSettings>().widget_pos(key);
    let rest = match saved {
        Some(at) => Rect::from_min_size(Pos2::new(at.x, at.y), size),
        None => corner.anchor_size(start, size),
    };
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

/// `None` once a closed window has lifted off entirely. A sheet comes to the front as it opens,
/// over whatever the player had up.
pub(crate) fn sheet_pose(
    world: &mut World,
    gui: &egui::Context,
    key: WindowKey,
    open: bool,
) -> Option<SheetPose> {
    let seconds = motion_step(gui);
    let mut clocks = world.resource_mut::<SheetClocks>();
    let state = clocks.0.entry(key).or_insert(SheetState {
        clock: if open {
            SheetClock::OPEN
        } else {
            SheetClock::CLOSED
        },
        open: false,
    });
    let opens = open && !state.open;
    state.open = open;
    state.clock.run(open, seconds);
    let clock = state.clock;
    if opens {
        gui.move_to_top(layer(key));
    }
    (!clock.is_closed() || open).then(|| clock.pose())
}

/// What each window of a view last drew, and where, kept for it to lift off showing.
#[derive(Resource, Default)]
pub(super) struct Afterimages(HashMap<WindowKey, Afterimage>);

struct Afterimage {
    image: float::Afterimage,
    rect: Rect,
    title: String,
}

fn motion_step(gui: &egui::Context) -> f32 {
    gui.input(|input| input.unstable_dt).min(MAX_STEP)
}

/// Where a window of one view stands: centred across the screen, `top` from its top.
#[derive(Clone, Copy)]
pub(crate) struct Centred {
    pub key: WindowKey,
    pub of: Placed,
    pub body: Vec2,
    pub top: f32,
}

/// What a window drew, and whether the player closed it.
pub(crate) struct Windowed<R> {
    pub inner: R,
    pub closed: bool,
}

/// A fixed-size window titled `title` that is up while `open` holds a view, and lifts off showing
/// the last it held. `None` once it has lifted off entirely; the player closing it is told by
/// `closed`, for its caller to answer.
pub(crate) fn centred<V, R>(
    world: &mut World,
    gui: &egui::Context,
    window: Centred,
    open: Option<V>,
    title: impl FnOnce(&V) -> String,
    add: impl FnOnce(&mut Ui, &mut World, &V) -> R,
) -> Option<Windowed<R>> {
    let screen = gui.content_rect();
    let size = sheet::titled_size(Vec2::new(
        window.body.x + 2.0 * sheet::PANE_PAD.x,
        window.body.y,
    ));
    let seat = Seat {
        key: window.key,
        of: window.of,
        start: Rect::from_min_size(
            Pos2::new(screen.center().x - size.x / 2.0, screen.top() + window.top),
            size,
        ),
        sizing: Sizing::Fixed,
    };
    viewed(world, gui, seat, open, title, add)
}

/// Where a window of one view starts until the player places it, and whether the player sizes it.
#[derive(Clone, Copy)]
pub(crate) struct Seat {
    pub key: WindowKey,
    pub of: Placed,
    pub start: Rect,
    pub sizing: Sizing,
}

/// A window titled `title` that is up while `open` holds a view, and lifts off showing what it
/// last drew. `None` once it has lifted off entirely, or while it does; the player closing it is
/// told by `closed`, for its caller to answer.
pub(crate) fn viewed<V, R>(
    world: &mut World,
    gui: &egui::Context,
    seat: Seat,
    open: Option<V>,
    title: impl FnOnce(&V) -> String,
    add: impl FnOnce(&mut Ui, &mut World, &V) -> R,
) -> Option<Windowed<R>> {
    let pose = sheet_pose(world, gui, seat.key, open.is_some())?;
    let Some(shown) = open else {
        lift_off(world, gui, seat.key, pose);
        return None;
    };
    let title = title(&shown);
    let frame = WindowFrame {
        key: seat.key,
        of: seat.of,
        title: Some(&title),
        start: seat.start,
        sizing: seat.sizing,
    };
    let drawn = window(world, gui, frame, pose, true, |ui, world| {
        add(ui, world, &shown)
    });
    if let Some(rect) = drawn_rect(gui, seat.key) {
        let image = float::Afterimage::of(gui, layer(seat.key));
        world
            .resource_mut::<Afterimages>()
            .0
            .insert(seat.key, Afterimage { image, rect, title });
    }
    Some(drawn)
}

fn lift_off(world: &mut World, gui: &egui::Context, key: WindowKey, pose: SheetPose) {
    mark_drawn(world, key);
    let Some(last) = world.resource::<Afterimages>().0.get(&key) else {
        return;
    };
    let pin = sheet::title_origin(gui, last.rect, &last.title);
    gui.set_transform_layer(layer(key), pose.transform(gui, last.rect, pin));
    egui::Area::new(panel_id(key))
        .order(Band::Hud.order())
        .fixed_pos(last.rect.min)
        .constrain(false)
        .fade_in(false)
        .interactable(false)
        .show(gui, |ui| {
            ui.multiply_opacity(pose.opacity);
            last.image.paint(ui.painter());
        });
}

/// A sheet window, up while `up` holds, drawn as `pose` has it. A window that is not up shows
/// what it last held, disabled, as it lifts off.
pub(crate) fn window<R>(
    world: &mut World,
    gui: &egui::Context,
    frame: WindowFrame,
    pose: SheetPose,
    up: bool,
    add: impl FnOnce(&mut Ui, &mut World) -> R,
) -> Windowed<R> {
    let WindowFrame {
        key,
        of,
        title,
        start,
        sizing,
    } = frame;
    let id = panel_id(key);
    let limits = sizing.limits(start);
    let kept = Kept::Placement(of);
    let mut closed = false;
    let rest = world
        .resource::<UserSettings>()
        .placement(of)
        .map(|placement| match sizing {
            Sizing::Fixed => Rect::from_min_size(placed_rect(placement).min, start.size()),
            Sizing::Resizable { .. } => placed_rect(placement),
        })
        .unwrap_or(start);
    let rect = standing(world, gui, key.name(), rest, limits);
    let front = front_sheet(world, gui) == Some(key);
    let at_rest = pose == SheetPose::REST;
    let shadow = match (at_rest, front) {
        (true, false) => sheet::shadow(Depth::Behind),
        _ => pose.shadow,
    };
    let pin = title.map_or(rect.min, |title| sheet::title_origin(gui, rect, title));
    let words = &messages::of(world.resource::<Content>()).interface;
    let (close, resize) = (words.close().to_owned(), words.resize().to_owned());
    gui.set_transform_layer(layer(key), pose.transform(gui, rect, pin));
    let shown = egui::Area::new(id)
        .order(Band::Hud.order())
        .fixed_pos(rect.min)
        .constrain(false)
        .fade_in(false)
        .show(gui, |ui| {
            ui.multiply_opacity(pose.opacity);
            if !up {
                ui.disable();
            }
            let (outer, _) = ui.allocate_exact_size(rect.size(), Sense::hover());
            let sheet = Sheet {
                title,
                closable: true,
                curl: matches!(sizing, Sizing::Resizable { .. }),
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
                && panel::close_button(ui, id, cross, &close).clicked()
            {
                closed = up;
            }
            let grip = parts
                .curl
                .map(|curl| panel::corner_grip(ui, id, curl, &resize));
            let inner = panel::body(ui, parts.body, |ui| add(ui, world));
            (handle, grip, inner)
        });
    let (handle, grip, inner) = shown.inner;
    let outer = shown.response.rect;
    grab(
        world,
        &handle,
        Held::new(key.name(), kept, Grip::Move, outer, limits),
    );
    if let Some(grip) = &grip {
        let resize = Grip::Resize(GripCorner::BottomRight);
        grab(
            world,
            grip,
            Held::new(key.name(), kept, resize, outer, limits),
        );
    }
    let mut drawn = world.resource_mut::<WindowsDrawn>();
    drawn.now.insert(key);
    if up {
        drawn.placed_now.push(PlacedWindow {
            of,
            rect: outer,
            band: handle.rect,
            corner: grip.as_ref().map(|grip| grip.rect),
        });
    }
    world
        .resource_mut::<gui::WidgetRects>()
        .0
        .insert(key.name().to_owned(), outer);
    Windowed { inner, closed }
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
    let limits = frame.sizing.limits(frame.start);
    let kept = Kept::Placement(frame.of);
    grab(
        world,
        &handle,
        Held::new(frame.key.name(), kept, Grip::Move, outer, limits),
    );
}

/// Where the window of `key` was last drawn, while it has been.
pub(crate) fn drawn_rect(gui: &egui::Context, key: WindowKey) -> Option<Rect> {
    gui.memory(|memory| memory.area_rect(panel_id(key)))
}

pub(crate) fn placed_spot(world: &World, gui: &egui::Context, key: WindowKey, start: Pos2) -> Rect {
    let saved = world
        .resource::<UserSettings>()
        .widget_pos(key.name())
        .map(|pos| Pos2::new(pos.x, pos.y));
    let rest = Rect::from_min_size(saved.unwrap_or(start), drawn_size(gui, panel_id(key)));
    standing(world, gui, key.name(), rest, Limits::NONE)
}

pub(crate) fn grab_spot(world: &mut World, key: &'static str, handle: &egui::Response, rect: Rect) {
    let held = Held::new(key, Kept::Spot, Grip::Move, rect, Limits::NONE);
    grab(world, handle, held);
}

pub(crate) fn front_sheet(world: &World, gui: &egui::Context) -> Option<WindowKey> {
    sheets_front_first(world, gui).first().copied()
}

/// The sheets drawn last frame as they are stacked, the one the player reached for last first.
pub(crate) fn sheets_front_first(world: &World, gui: &egui::Context) -> Vec<WindowKey> {
    let drawn = &world.resource::<WindowsDrawn>().last;
    let mut sheets: Vec<WindowKey> = gui.memory(|memory| {
        memory
            .layer_ids()
            .filter_map(|layer_id| drawn.iter().copied().find(|key| layer(*key) == layer_id))
            .collect()
    });
    sheets.reverse();
    sheets
}

pub(crate) fn mark_drawn(world: &mut World, key: WindowKey) {
    world.resource_mut::<WindowsDrawn>().now.insert(key);
}

pub(crate) fn layer(key: WindowKey) -> LayerId {
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
pub(super) struct SheetClocks(HashMap<WindowKey, SheetState>);

/// A sheet's clock, and whether it was open as it last ran.
struct SheetState {
    clock: SheetClock,
    open: bool,
}

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

pub(crate) fn panel_id(key: WindowKey) -> Id {
    Id::new(("hud", key.name()))
}

fn widget_id(key: &str) -> Id {
    Id::new(("hud widget", key))
}
