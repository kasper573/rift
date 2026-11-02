use std::borrow::Cow;
use std::marker::PhantomData;
use std::time::Duration;
use ui::dusk;
use ui::egui;

use bevy::ecs::component::Mutable;
use bevy::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ui::egui::emath::TSTransform;
use ui::egui::{
    Align, Color32, FontId, Id, LayerId, Layout, Order, Rect, RichText, Sense, Ui, UiBuilder, Vec2,
};
use ui::face::Face;
use ui::ink::Ground;

use super::{NotificationSent, window_visible};
use crate::core::gui::{self, Band, Tier};
use crate::core::text::{self, KeyNames, StyledLine, TextLook, TextPiece};
use crate::core::units::Seconds;
use crate::systems::GameSettings;
use crate::systems::input::map::{self, InputAction};
use crate::systems::screen::ClientScreen;
use ui::motion::{Transit, TransitMove, TransitPose, Transits};
use ui::paint;

const WHITE: Color32 = Color32::WHITE;
pub(super) const SLATE_90: Color32 = Color32::from_rgb(228, 234, 239);
pub(super) const SLATE_40: Color32 = Color32::from_rgb(41, 46, 45);
const AMBER_80: Color32 = Color32::from_rgb(246, 201, 96);
const LABEL: f32 = 12.0;
const CAPTION: f32 = 12.0;

const CAPTIONS_WIDEST: f32 = 640.0;
const CAPTION_SIZE: f32 = 17.0;
const CAPTION_LINES_HIGH: f32 = 46.0;
const CAPTION_PADDING: Vec2 = Vec2::new(24.0, 8.0);
const MILESTONE_SIZE: f32 = 34.0;
const SCREEN_SHARE: f32 = 0.94;
const LINE_GAP: f32 = 2.0;
const ROW_GAP: f32 = 4.0;
const CHIP_GAP: f32 = 8.0;
const MILESTONE_GAP: f32 = 16.0;

const SCRIM_TRANSIT: Transit = Transit {
    enter: TransitMove::new(Vec2::new(0.0, 12.0), 0.97, 0.36),
    exit: TransitMove::new(Vec2::new(0.0, 8.0), 0.98, 0.24),
};
const CAPTION_TRANSIT: Transit = Transit {
    enter: TransitMove::new(Vec2::new(0.0, 8.0), 1.0, 0.34),
    exit: TransitMove::new(Vec2::new(0.0, -6.0), 1.0, 0.22),
};
const MILESTONE_TRANSIT: Transit = Transit {
    enter: TransitMove::new(Vec2::ZERO, 1.25, 0.52),
    exit: TransitMove::new(Vec2::new(0.0, -14.0), 1.04, 0.35),
};

pub(super) trait RowPlace: Send + Sync + 'static {
    const ROWS: usize;
}

pub(super) struct Captions;
pub(super) struct Milestones;

impl RowPlace for Captions {
    const ROWS: usize = 3;
}

impl RowPlace for Milestones {
    const ROWS: usize = 2;
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

/// A place on screen notices show in: its clock moves while the game window is on screen and,
/// where hovering holds it, while the pointer is off it, and it lets go of what lapsed by it.
pub(super) trait NoticePlace: Resource<Mutability = Mutable> + Default {
    /// Whether hovering what the place drew holds its notices.
    const HOLDS: bool = true;

    fn drawn(&self) -> &[Rect];

    /// Moves the place's clock on by `delta` and lets go of what lapsed, a newer notice taking an
    /// older one's place once that has shown for `glance`.
    fn advance(&mut self, delta: Duration, glance: Duration);
}

/// Keeps a place of notices: aging it in an area, and emptying it when the area is left, as all a
/// place keeps is about the area it was raised in.
pub(super) fn place<P: NoticePlace>(app: &mut App) {
    app.init_resource::<P>()
        .add_systems(
            Update,
            age::<P>
                .run_if(in_state(ClientScreen::Area))
                .run_if(window_visible),
        )
        .add_systems(OnExit(ClientScreen::Area), |world: &mut World| {
            world.insert_resource(P::default());
        });
}

fn age<P: NoticePlace>(world: &mut World) {
    if P::HOLDS && hovered(world, world.resource::<P>().drawn()) {
        return;
    }
    let delta = world.resource::<Time>().delta();
    let glance = world.resource::<GameSettings>().reading.glance.into();
    world.resource_mut::<P>().advance(delta, glance);
}

impl<P: RowPlace> NoticePlace for PlacedRows<P> {
    fn drawn(&self) -> &[Rect] {
        &self.drawn
    }

    fn advance(&mut self, delta: Duration, glance: Duration) {
        self.rows.tick(delta, glance);
    }
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

pub(super) fn draw_captions(world: &mut World, gui: &egui::Context, top: f32) -> f32 {
    let now = time(gui);
    let (lines, more) = lines_of::<Captions>(world);
    let mut placed = world.resource_mut::<PlacedRows<Captions>>();
    let scrim = placed
        .scrim
        .sync(now, SCRIM_TRANSIT, (!lines.is_empty()).then_some((0, ())));
    let rows = placed.fades.sync(
        now,
        CAPTION_TRANSIT,
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
    let measured: Vec<(LabelledLine, TransitPose, Vec2)> = rows
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
        MILESTONE_TRANSIT,
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
    fades: Transits<LabelledLine>,
    scrim: Transits<()>,
    place: PhantomData<fn() -> P>,
}

impl<P: RowPlace> Default for PlacedRows<P> {
    fn default() -> PlacedRows<P> {
        PlacedRows {
            rows: VoiceRows::new(P::ROWS),
            drawn: Vec::new(),
            fades: Transits::default(),
            scrim: Transits::default(),
            place: PhantomData,
        }
    }
}

struct VoiceRows {
    limit: usize,
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
    fn new(limit: usize) -> VoiceRows {
        VoiceRows {
            limit,
            rows: Vec::new(),
            turned_away: None,
            clock: Clock::default(),
            keys: 0,
        }
    }

    fn join(&mut self, voice: Cow<'static, str>, notification_sent: NotificationSent) {
        let now = self.clock.now();
        let stays: Duration = notification_sent.stays.into();
        if let Some(row) = self.rows.iter_mut().find(|row| row.voice == voice) {
            if row.shown.notification == notification_sent.notification {
                row.read_by = row.read_by.max(now + stays);
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
            turned_away.until = turned_away.until.max(now + stays);
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

    /// Moves the rows along, a newer line replacing a row's once it has shown for `glance`.
    fn tick(&mut self, delta: Duration, glance: Duration) {
        self.clock.advance(delta);
        let now = self.clock.now();
        for row in &mut self.rows {
            if now >= row.changed + glance
                && let Some(newest) = row.newest.take()
            {
                let stays: Duration = newest.stays.into();
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

fn milestone_row(ui: &mut Ui, world: &World, line: &LabelledLine, row: TransitPose) -> Rect {
    ui.spacing_mut().item_spacing.y = LINE_GAP;
    let shift = TSTransform::new(row.offset, row.scale);
    let label = text::show_shadowed(
        ui,
        &label_text(&line.label, line.replaced),
        label_font(),
        WHITE.gamma_multiply(0.85),
        dusk::TEXT_SHADOW,
        shift,
    );
    let words = line.text.plain(world.resource::<KeyNames>());
    let words = text::show_shadowed(
        ui,
        &words,
        Face::BookBold.id(MILESTONE_SIZE),
        AMBER_80,
        dusk::TEXT_SHADOW,
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
