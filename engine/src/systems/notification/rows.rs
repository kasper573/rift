use std::borrow::Cow;
use std::marker::PhantomData;
use std::time::Duration;
use ui::egui;

use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ui::dusk;
use ui::egui::emath::{TSTransform, easing};
use ui::egui::{
    Align, Align2, Color32, FontId, Id, LayerId, Layout, Order, Rect, RichText, Sense, Ui,
    UiBuilder, Vec2,
};
use ui::face::Face;
use ui::glyph::Mark;
use ui::ink::Ground;

use super::{NotificationSent, window_visible};
use crate::core::gui::{self, Band, Tier};
use crate::core::text::{self, KeyNames, StyledLine, TextLook, TextPiece, TextShadow};
use crate::core::time::Seconds;
use crate::systems::input::map::{self, InputAction};
use crate::systems::screen::ClientScreen;
use ui::form;
use ui::paint;

const COUNT_LINGERS: Seconds = Seconds(5.0);

const WHITE: Color32 = Color32::WHITE;
pub(super) const SLATE_90: Color32 = Color32::from_rgb(228, 234, 239);
pub(super) const SLATE_40: Color32 = Color32::from_rgb(41, 46, 45);
const AMBER_80: Color32 = Color32::from_rgb(246, 201, 96);
const LABEL: f32 = 12.0;
const CLOSE: f32 = 20.0;
const CAPTION: f32 = 12.0;
const BODY: f32 = 16.0;

const CAPTIONS_WIDEST: f32 = 640.0;
const CAPTION_SIZE: f32 = 17.0;
const CAPTION_LINES_HIGH: f32 = 46.0;
const CAPTION_PADDING: Vec2 = Vec2::new(24.0, 8.0);
const ALERTS_WIDEST: f32 = 580.0;
const MILESTONE_SIZE: f32 = 34.0;
const SCREEN_SHARE: f32 = 0.94;
const LINE_GAP: f32 = 2.0;
const ROW_GAP: f32 = 4.0;
const CHIP_GAP: f32 = 8.0;
const MILESTONE_GAP: f32 = 16.0;

const SCRIM_PRESENCE: Presence = Presence {
    enter: Move::new(Vec2::new(0.0, 12.0), 0.97, 0.36),
    exit: Move::new(Vec2::new(0.0, 8.0), 0.98, 0.24),
};
const CAPTION_PRESENCE: Presence = Presence {
    enter: Move::new(Vec2::new(0.0, 8.0), 1.0, 0.34),
    exit: Move::new(Vec2::new(0.0, -6.0), 1.0, 0.22),
};
const MILESTONE_PRESENCE: Presence = Presence {
    enter: Move::new(Vec2::ZERO, 1.25, 0.52),
    exit: Move::new(Vec2::new(0.0, -14.0), 1.04, 0.35),
};
const SHADOW: TextShadow = TextShadow {
    offset: Vec2::new(0.0, 2.0),
    color: Color32::from_black_alpha(204),
};

pub(super) trait RowPlace: Send + Sync + 'static {
    const ROWS: usize;
    const REPLACE_AFTER: Seconds;
}

pub(super) struct Captions;
pub(super) struct Alerts;
pub(super) struct Milestones;

impl RowPlace for Captions {
    const ROWS: usize = 3;
    const REPLACE_AFTER: Seconds = Seconds(2.0);
}

impl RowPlace for Alerts {
    const ROWS: usize = 2;
    const REPLACE_AFTER: Seconds = Seconds(3.0);
}

impl RowPlace for Milestones {
    const ROWS: usize = 2;
    const REPLACE_AFTER: Seconds = Seconds(2.0);
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct NotificationRows {
    pub rows: Vec<NotificationRow>,
    pub more: u32,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct NotificationRow {
    pub label: String,
    pub text: String,
    pub replaced: u32,
}

pub(super) fn plugin<P: RowPlace>(app: &mut App) {
    app.init_resource::<PlacedRows<P>>()
        .add_systems(
            Update,
            tick::<P>
                .run_if(in_state(ClientScreen::Area))
                .run_if(window_visible),
        )
        .add_systems(OnExit(ClientScreen::Area), forget::<PlacedRows<P>>);
}

/// Everything a notification place or row keeps is about the area it was raised in, so each one
/// starts empty when that area is left.
pub(super) fn forget<R: Resource + Default>(world: &mut World) {
    world.insert_resource(R::default());
}

pub(super) fn join<P: RowPlace>(
    world: &mut World,
    voice: Cow<'static, str>,
    notification_sent: NotificationSent,
) {
    world
        .resource_mut::<PlacedRows<P>>()
        .rows
        .join(voice, notification_sent);
}

pub(super) fn shown<P: RowPlace>(world: &World) -> NotificationRows {
    world.resource::<PlacedRows<P>>().rows.shown()
}

pub(super) fn draw_alerts(world: &mut World, gui: &egui::Context, top: f32) -> f32 {
    let (lines, more) = lines_of::<Alerts>(world);
    let width = ALERTS_WIDEST.min(gui.content_rect().width() * SCREEN_SHARE);
    let mut drawn: Vec<Rect> = Vec::new();
    let next = |drawn: &[Rect]| drawn.last().map_or(top, |last| last.bottom() + ROW_GAP);
    for line in lines {
        let area = Stacking::OverWindows
            .area(gui, Id::new(("alert", line.key)))
            .anchor(Align2::CENTER_TOP, Vec2::new(0.0, next(&drawn)));
        let shown = area.show(gui, |ui| {
            dusk::frame().show(ui, |ui| {
                ui.set_width(width);
                let closed = alert_title(ui, &label_text(&line.label, line.replaced));
                text::show(ui, world, &line.text, look(BODY, dusk::TEXT), None);
                closed
            })
        });
        drawn.push(shown.response.rect);
        if shown.inner.inner {
            world
                .resource_mut::<PlacedRows<Alerts>>()
                .rows
                .close(line.key);
        }
    }
    if let Some(more) = more {
        let at = next(&drawn);
        drawn.push(chip_area(
            world,
            gui,
            "alerts-more",
            at,
            more,
            Stacking::OverWindows,
        ));
    }
    let bottom = drawn.last().map_or(top, Rect::bottom);
    world.resource_mut::<PlacedRows<Alerts>>().drawn = drawn;
    bottom
}

pub(super) fn draw_captions(world: &mut World, gui: &egui::Context, top: f32) -> f32 {
    let now = time(gui);
    let (lines, more) = lines_of::<Captions>(world);
    let mut placed = world.resource_mut::<PlacedRows<Captions>>();
    let scrim = placed
        .scrim
        .sync(now, SCRIM_PRESENCE, (!lines.is_empty()).then_some((0, ())));
    let rows = placed.fades.sync(
        now,
        CAPTION_PRESENCE,
        lines.into_iter().map(|line| (line.key, line)),
    );
    let Some(&((), scrim)) = scrim.first() else {
        let drawn: Vec<Rect> = more
            .map(|more| {
                chip_area(
                    world,
                    gui,
                    "captions-more",
                    top,
                    more,
                    Stacking::UnderWindows,
                )
            })
            .into_iter()
            .collect();
        let bottom = drawn.first().map_or(top, Rect::bottom);
        world.resource_mut::<PlacedRows<Captions>>().drawn = drawn;
        return bottom;
    };
    let screen = gui.content_rect();
    let room =
        (CAPTIONS_WIDEST.min(screen.width() * SCREEN_SHARE) - 2.0 * CAPTION_PADDING.x).max(0.0);
    let measured: Vec<(LabelledLine, Look, Vec2)> = rows
        .into_iter()
        .map(|(line, row)| {
            let size = text::measure(gui, world, &line.text, look(CAPTION_SIZE, WHITE), room);
            (line, row, size)
        })
        .collect();
    let width = measured
        .iter()
        .map(|(line, _, size)| {
            let label = label_width(gui, &label_text(&line.label, line.replaced));
            size.x.max(label)
        })
        .fold(0.0, f32::max)
        .min(room);
    let shown = column_area(gui, "captions", top, Stacking::UnderWindows).show(gui, |ui| {
        ui.set_width(screen.width());
        ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing.y = ROW_GAP;
            let scrim_rect = egui::Frame::new()
                .fill(Color32::from_black_alpha(115))
                .corner_radius(8.0)
                .inner_margin(egui::Margin::symmetric(
                    CAPTION_PADDING.x as i8,
                    CAPTION_PADDING.y as i8,
                ))
                .show(ui, |ui| {
                    ui.set_width(width);
                    ui.spacing_mut().item_spacing.y = CHIP_GAP;
                    for (line, row, size) in &measured {
                        ui.scope(|ui| {
                            ui.multiply_opacity(row.opacity);
                            caption_row(ui, world, line, *size);
                        });
                    }
                })
                .response
                .rect;
            let chip = more.map(|more| count_chip(ui, world, more).rect);
            chip.map_or(scrim_rect, |chip| scrim_rect.union(chip))
        })
        .inner
    });
    gui.set_transform_layer(shown.response.layer_id, scrim.around(shown.inner));
    world.resource_mut::<PlacedRows<Captions>>().drawn = vec![shown.inner];
    shown.inner.bottom()
}

pub(super) fn draw_milestones(world: &mut World, gui: &egui::Context, top: f32) -> f32 {
    let now = time(gui);
    let (lines, more) = lines_of::<Milestones>(world);
    let rows = world.resource_mut::<PlacedRows<Milestones>>().fades.sync(
        now,
        MILESTONE_PRESENCE,
        lines.into_iter().map(|line| (line.key, line)),
    );
    if rows.is_empty() && more.is_none() {
        world.resource_mut::<PlacedRows<Milestones>>().drawn.clear();
        return top;
    }
    let screen = gui.content_rect();
    let shown = column_area(gui, "milestones", top, Stacking::OverNotices).show(gui, |ui| {
        ui.set_width(screen.width());
        ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing.y = MILESTONE_GAP;
            let mut drawn: Option<Rect> = None;
            for (line, row) in &rows {
                let rect = ui
                    .scope(|ui| {
                        ui.multiply_opacity(row.opacity);
                        milestone_row(ui, world, line, *row)
                    })
                    .inner;
                drawn = Some(drawn.map_or(rect, |drawn| drawn.union(rect)));
            }
            if let Some(more) = more {
                ui.add_space(CHIP_GAP - MILESTONE_GAP);
                let chip = count_chip(ui, world, more).rect;
                drawn = Some(drawn.map_or(chip, |drawn| drawn.union(chip)));
            }
            drawn
        })
        .inner
    });
    let drawn: Vec<Rect> = shown.inner.into_iter().collect();
    let bottom = drawn.first().map_or(top, Rect::bottom);
    world.resource_mut::<PlacedRows<Milestones>>().drawn = drawn;
    bottom
}

pub(super) fn count_chip(ui: &mut Ui, world: &World, more: u32) -> egui::Response {
    egui::Frame::new()
        .fill(SLATE_40.gamma_multiply(0.55))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(8, 2))
        .show(ui, |ui| {
            let size = text::measure(
                ui.ctx(),
                world,
                &more_text(more),
                look(CAPTION, SLATE_90.gamma_multiply(0.75)),
                f32::INFINITY,
            );
            ui.set_width(size.x);
            text::show(
                ui,
                world,
                &more_text(more),
                look(CAPTION, SLATE_90.gamma_multiply(0.75)),
                None,
            );
        })
        .response
}

pub(super) fn more_text(more: u32) -> StyledLine {
    StyledLine::new(vec![
        TextPiece::text(format!("+{more} more \u{b7} ")),
        map::input(InputAction::ToggleHistory),
        TextPiece::text(" history"),
    ])
}

pub(super) fn hovered(world: &World, rects: &[Rect]) -> bool {
    let pointer = gui::context(world).and_then(|gui| gui.pointer_latest_pos());
    pointer.is_some_and(|at| rects.iter().any(|rect| rect.contains(at)))
}

const BUMP_SCALE: f32 = 1.08;
const BUMP_SECONDS: f64 = 0.25;

/// The time a notification place has been on show. It only moves while the place is not held,
/// so a notification read under the pointer keeps its time.
#[derive(Default, Clone, Copy)]
pub(super) struct Clock(Duration);

impl Clock {
    pub(super) fn advance(&mut self, delta: Duration) {
        self.0 += delta;
    }

    pub(super) fn now(self) -> Duration {
        self.0
    }

    pub(super) fn ends_after(self, stays: Seconds) -> Duration {
        self.0 + Duration::from(stays)
    }

    pub(super) fn lapsed(self, deadline: Duration) -> bool {
        self.0 >= deadline
    }
}

/// A toast's swell after a repeat bumps it: armed by the bump, timed from the next draw.
#[derive(Default, Clone, Copy)]
pub(super) struct Swell {
    bumped: bool,
    started: Option<f64>,
}

impl Swell {
    pub(super) fn bump(&mut self) {
        self.bumped = true;
    }

    pub(super) fn start_if_bumped(&mut self, now: f64) {
        if std::mem::take(&mut self.bumped) {
            self.started = Some(now);
        }
    }

    /// How far the toast is scaled now, easing back to its size.
    pub(super) fn scale(self, now: f64) -> f32 {
        self.started.map_or(1.0, |at| {
            let t = ((now - at) / BUMP_SECONDS).clamp(0.0, 1.0) as f32;
            1.0 + (BUMP_SCALE - 1.0) * (1.0 - easing::cubic_out(t))
        })
    }
}

pub(super) fn time(gui: &egui::Context) -> f64 {
    gui.input(|input| input.time)
}

pub(super) fn look(size: f32, color: Color32) -> TextLook {
    TextLook::new(size, color).on(Ground::Dusk)
}

pub(super) fn label_font() -> FontId {
    Face::BookBold.id(LABEL)
}

/// Where a notice stacks among what else is on screen.
#[derive(Clone, Copy)]
pub(super) enum Stacking {
    /// Over every window and card: an error or an alert the player must not miss.
    OverWindows,
    /// Over the world, the HUD and the stage, but under the windows and cards the player has open.
    UnderWindows,
    /// Under the windows too, but over every notice stacked `UnderWindows`: a milestone is never
    /// hidden by what else is being told at the time.
    OverNotices,
}

impl Stacking {
    pub(super) fn area(self, gui: &egui::Context, id: Id) -> egui::Area {
        match self {
            Stacking::OverWindows => egui::Area::new(id).order(Band::Notices.order()),
            Stacking::UnderWindows => Stacking::in_tier(gui, Tier::Top, id),
            Stacking::OverNotices => Stacking::in_tier(gui, Tier::Peak, id),
        }
    }

    fn in_tier(gui: &egui::Context, tier: Tier, id: Id) -> egui::Area {
        gui::stack(gui, tier, LayerId::new(Order::Background, id));
        egui::Area::new(id).order(Order::Background)
    }
}

pub(super) fn column_area(
    gui: &egui::Context,
    id: &str,
    top: f32,
    stacking: Stacking,
) -> egui::Area {
    stacking
        .area(gui, Id::new(("notice", id)))
        .interactable(false)
        .fade_in(false)
        .fixed_pos(egui::pos2(gui.content_rect().left(), top))
}

pub(super) struct Fades<T> {
    items: Vec<Fading<T>>,
}

impl<T> Default for Fades<T> {
    fn default() -> Fades<T> {
        Fades { items: Vec::new() }
    }
}

impl<T: Clone> Fades<T> {
    /// Brings the place up to what it shows now, in the order given. An item that just left keeps
    /// the slot it had while it fades out.
    pub(super) fn sync(
        &mut self,
        now: f64,
        presence: Presence,
        current: impl IntoIterator<Item = (u64, T)>,
    ) -> Vec<(T, Look)> {
        let mut previous: Vec<Option<Fading<T>>> = std::mem::take(&mut self.items)
            .into_iter()
            .map(Some)
            .collect();
        let mut items: Vec<Fading<T>> = Vec::new();
        for (key, item) in current {
            let kept = previous
                .iter_mut()
                .find_map(|slot| slot.take_if(|fading| fading.key == key));
            items.push(match kept {
                Some(mut fading) => {
                    fading.item = item;
                    if fading.leaving {
                        fading.leaving = false;
                        fading.since = now;
                    }
                    fading
                }
                None => Fading {
                    key,
                    item,
                    since: now,
                    leaving: false,
                },
            });
        }
        for (index, slot) in previous.into_iter().enumerate() {
            let Some(mut fading) = slot else {
                continue;
            };
            if !fading.leaving {
                fading.leaving = true;
                fading.since = now;
            }
            if now - fading.since < presence.exit.seconds {
                items.insert(index.min(items.len()), fading);
            }
        }
        self.items = items;
        self.items
            .iter()
            .map(|fading| (fading.item.clone(), fading.look(now, presence)))
            .collect()
    }
}

#[derive(Clone, Copy)]
pub(super) struct Presence {
    pub enter: Move,
    pub exit: Move,
}

#[derive(Clone, Copy)]
pub(super) struct Move {
    offset: Vec2,
    scale: f32,
    seconds: f64,
}

impl Move {
    pub(super) const fn new(offset: Vec2, scale: f32, seconds: f64) -> Move {
        Move {
            offset,
            scale,
            seconds,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Look {
    pub opacity: f32,
    pub offset: Vec2,
    pub scale: f32,
}

impl Look {
    /// The transform that gives whatever is drawn in `rect` this look, scaled about its centre.
    pub(super) fn around(self, rect: Rect) -> TSTransform {
        let centre = rect.center().to_vec2();
        TSTransform::new(centre * (1.0 - self.scale) + self.offset, self.scale)
    }
}

#[derive(Clone, PartialEq)]
pub(super) struct LabelledLine {
    pub key: u64,
    pub label: String,
    pub replaced: u32,
    pub text: StyledLine,
}

#[derive(Resource)]
pub(super) struct PlacedRows<P> {
    rows: VoiceRows,
    drawn: Vec<Rect>,
    fades: Fades<LabelledLine>,
    scrim: Fades<()>,
    place: PhantomData<fn() -> P>,
}

impl<P: RowPlace> Default for PlacedRows<P> {
    fn default() -> PlacedRows<P> {
        PlacedRows {
            rows: VoiceRows::new(P::ROWS, P::REPLACE_AFTER),
            drawn: Vec::new(),
            fades: Fades::default(),
            scrim: Fades::default(),
            place: PhantomData,
        }
    }
}

struct Fading<T> {
    key: u64,
    item: T,
    since: f64,
    leaving: bool,
}

impl<T> Fading<T> {
    fn look(&self, now: f64, presence: Presence) -> Look {
        let (step, away) = if self.leaving {
            let t = ((now - self.since) / presence.exit.seconds).clamp(0.0, 1.0) as f32;
            (presence.exit, easing::cubic_in(t))
        } else {
            let t = ((now - self.since) / presence.enter.seconds).clamp(0.0, 1.0) as f32;
            (presence.enter, 1.0 - easing::cubic_out(t))
        };
        Look {
            opacity: 1.0 - away,
            offset: step.offset * away,
            scale: 1.0 + (step.scale - 1.0) * away,
        }
    }
}

impl Default for Look {
    fn default() -> Look {
        Look {
            opacity: 1.0,
            offset: Vec2::ZERO,
            scale: 1.0,
        }
    }
}

struct VoiceRows {
    limit: usize,
    replace_after: Duration,
    rows: Vec<VoiceRow>,
    turned_away: Option<TurnedAway>,
    clock: Clock,
    keys: u64,
}

struct VoiceRow {
    key: u64,
    voice: Cow<'static, str>,
    shown: NotificationSent,
    newest: Option<NotificationSent>,
    replaced: u32,
    changed: Duration,
    read_by: Duration,
}

struct TurnedAway {
    more: u32,
    until: Duration,
}

impl VoiceRows {
    fn new(limit: usize, replace_after: Seconds) -> VoiceRows {
        VoiceRows {
            limit,
            replace_after: replace_after.into(),
            rows: Vec::new(),
            turned_away: None,
            clock: Clock::default(),
            keys: 0,
        }
    }

    fn join(&mut self, voice: Cow<'static, str>, notification_sent: NotificationSent) {
        let now = self.clock.now();
        let stays: Duration = notification_sent.notification.stays().into();
        if let Some(row) = self.rows.iter_mut().find(|row| row.voice == voice) {
            if row.shown.notification == notification_sent.notification {
                row.read_by = row.read_by.max(
                    self.clock
                        .ends_after(notification_sent.notification.stays()),
                );
                return;
            }
            row.replaced += 1;
            row.newest = Some(notification_sent);
            return;
        }
        if self.rows.len() >= self.limit {
            let turned_away = self.turned_away.get_or_insert(TurnedAway {
                more: 0,
                until: now,
            });
            turned_away.more += 1;
            turned_away.until = self.clock.ends_after(COUNT_LINGERS);
            return;
        }
        let after = self.rows.last().map_or(now, |last| last.read_by.max(now));
        self.keys += 1;
        self.rows.push(VoiceRow {
            key: self.keys,
            voice,
            shown: notification_sent,
            newest: None,
            replaced: 0,
            changed: now,
            read_by: after + stays,
        });
    }

    fn tick(&mut self, delta: Duration, held: bool) {
        if held {
            return;
        }
        self.clock.advance(delta);
        let now = self.clock.now();
        for row in &mut self.rows {
            if now >= row.changed + self.replace_after
                && let Some(newest) = row.newest.take()
            {
                let stays: Duration = newest.notification.stays().into();
                row.read_by = row.read_by.max(now + stays);
                row.shown = newest;
                row.changed = now;
            }
        }
        self.rows.retain(|row| row.read_by > now);
        self.turned_away = self
            .turned_away
            .take()
            .filter(|turned_away| turned_away.until > now);
    }

    fn close(&mut self, key: u64) {
        self.rows.retain(|row| row.key != key);
    }

    fn labelled_lines(&self) -> Vec<LabelledLine> {
        self.rows
            .iter()
            .map(|row| LabelledLine {
                key: row.key,
                label: row.voice.to_string(),
                replaced: row.replaced,
                text: row.shown.notification.text.styled(),
            })
            .collect()
    }

    fn more(&self) -> Option<u32> {
        self.turned_away
            .as_ref()
            .map(|turned_away| turned_away.more)
    }

    fn shown(&self) -> NotificationRows {
        NotificationRows {
            rows: self
                .rows
                .iter()
                .map(|row| NotificationRow {
                    label: row.voice.to_string(),
                    text: row.shown.notification.text.plain_words(),
                    replaced: row.replaced,
                })
                .collect(),
            more: self.more().unwrap_or(0),
        }
    }
}

fn tick<P: RowPlace>(world: &mut World) {
    let held = hovered(world, &world.resource::<PlacedRows<P>>().drawn);
    let delta = world.resource::<Time>().delta();
    world.resource_mut::<PlacedRows<P>>().rows.tick(delta, held);
}

fn lines_of<P: RowPlace>(world: &World) -> (Vec<LabelledLine>, Option<u32>) {
    let placed = world.resource::<PlacedRows<P>>();
    (placed.rows.labelled_lines(), placed.rows.more())
}

fn chip_area(
    world: &mut World,
    gui: &egui::Context,
    id: &str,
    top: f32,
    more: u32,
    stacking: Stacking,
) -> Rect {
    let width = gui.content_rect().width();
    column_area(gui, id, top, stacking)
        .show(gui, |ui| {
            ui.set_width(width);
            ui.vertical_centered(|ui| count_chip(ui, world, more).rect)
                .inner
        })
        .inner
}

fn caption_row(ui: &mut Ui, world: &World, line: &LabelledLine, size: Vec2) {
    ui.spacing_mut().item_spacing.y = LINE_GAP;
    ui.label(
        RichText::new(label_text(&line.label, line.replaced))
            .font(label_font())
            .color(WHITE.gamma_multiply(0.7)),
    );
    let clipped = Vec2::new(size.x, size.y.min(CAPTION_LINES_HIGH));
    let (rect, _) = ui.allocate_exact_size(clipped, Sense::hover());
    let mut text_ui = ui.new_child(
        UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::top_down(Align::Min)),
    );
    text_ui.set_clip_rect(rect.intersect(ui.clip_rect()));
    text::show(
        &mut text_ui,
        world,
        &line.text,
        look(CAPTION_SIZE, WHITE),
        None,
    );
}

fn milestone_row(ui: &mut Ui, world: &World, line: &LabelledLine, row: Look) -> Rect {
    ui.spacing_mut().item_spacing.y = LINE_GAP;
    let shift = TSTransform::new(row.offset, row.scale);
    let label = text::show_shadowed(
        ui,
        &label_text(&line.label, line.replaced),
        label_font(),
        WHITE.gamma_multiply(0.85),
        SHADOW,
        shift,
    );
    let words = line.text.plain(world.resource::<KeyNames>());
    let words = text::show_shadowed(
        ui,
        &words,
        Face::BookBold.id(MILESTONE_SIZE),
        AMBER_80,
        SHADOW,
        shift,
    );
    label.rect.union(words.rect)
}

fn label_text(label: &str, replaced: u32) -> String {
    match replaced {
        0 => label.to_uppercase(),
        replaced => format!("{} +{replaced}", label.to_uppercase()),
    }
}

fn label_width(gui: &egui::Context, label: &str) -> f32 {
    paint::text_size(gui, label, label_font()).x
}

/// The alert's label over a rule, with the cross that closes it; true once the cross is clicked.
fn alert_title(ui: &mut Ui, label: &str) -> bool {
    let closed = ui
        .horizontal(|ui| {
            ui.label(RichText::new(label).font(label_font()).color(dusk::TEXT));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (rect, response) = ui.allocate_exact_size(Vec2::splat(CLOSE), Sense::click());
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Close")
                });
                if response.hovered() {
                    ui.painter().rect_filled(rect, 4.0, dusk::HOVER);
                }
                Mark::Cross.paint(ui.painter(), rect.shrink(CLOSE * 0.2), dusk::TEXT);
                response.clicked()
            })
            .inner
        })
        .inner;
    form::rule_with(ui, egui::Stroke::new(1.0, dusk::BORDER));
    ui.add_space(6.0);
    closed
}
