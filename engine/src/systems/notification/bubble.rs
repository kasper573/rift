use std::time::Duration;

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::EguiPrimaryContextPass;
use egui::{Align, Color32, Layout, Sense, Shape, Stroke, StrokeKind, UiBuilder};

use super::rows::{self, Clock, Fades, Look, Move, Presence};
use super::{Notification, NotificationKind, NotificationSent, window_visible};
use crate::core::audio::playback::SfxPlace;
use crate::core::babble::{BabbleId, BabbleRank, Babbler, Babbling};
use crate::core::content::Content;
use crate::core::gui::{self, Band};
use crate::core::math::Pos;
use crate::core::render::tile_to_window;
use crate::core::text::{self, StyledLine, Typewriter};
use crate::core::tiling::{TilePos, Tiles};
use crate::core::time::Seconds;
use crate::systems::actor::{Action, Actor, plate};
use crate::systems::combat::Attitude;
use crate::systems::dialogue::stage;
use crate::systems::hud;
use crate::systems::movement::{Position, RenderPosition};
use crate::systems::npc::NpcId;
use crate::systems::player::session;
use crate::systems::screen::ClientScreen;

const LINES: usize = 2;
const FULL: usize = 3;
const FULL_IN_SMALL_WINDOW: usize = 2;
const SMALL_WINDOW: f32 = 960.0;
const REPLACE_AFTER: Seconds = Seconds(2.0);
const READ_LINGERS: Seconds = Seconds(0.6);
const FIGHTING_WITHIN: Tiles = Tiles(3.0);
const EDGE: f32 = 8.0;
const CROWN_GAP: f32 = 4.0;

const WIDEST: f32 = 290.0;
const LINE_SIZE: f32 = 15.0;
const LINES_HIGH: f32 = 63.0;
const PADDING: egui::Vec2 = egui::vec2(10.0, 6.0);
const BORDER: f32 = 1.0;
const RADIUS: f32 = 8.0;
const PART_GAP: f32 = 2.0;
const LINE_GAP: f32 = 4.0;
const TAIL: f32 = 12.0;
const TAIL_OUT: f32 = TAIL / 2.0;
const READ_OPACITY: f32 = 0.5;
const INK: Color32 = Color32::from_rgb(228, 234, 239);
const FILL: Color32 = Color32::from_rgb(41, 46, 45);
const OUTLINE: Color32 = Color32::from_rgb(73, 74, 80);
// The offsets are those of a bubble whose tail points down; `Tail::turn` turns them to its tail.
const PRESENCE: Presence = Presence {
    enter: Move::new(egui::vec2(0.0, 10.0), 0.82, 0.32),
    exit: Move::new(egui::vec2(0.0, -8.0), 0.94, 0.2),
};

pub struct BubblePlugin;

impl Plugin for BubblePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Bubbles>()
            .add_systems(
                Update,
                tick.run_if(in_state(ClientScreen::Area))
                    .run_if(window_visible),
            )
            .add_systems(
                EguiPrimaryContextPass,
                place
                    .after(plate::draw_plates)
                    .in_set(Band::World)
                    .run_if(in_state(ClientScreen::Area)),
            )
            .add_systems(OnExit(ClientScreen::Area), forget);
    }
}

pub fn say(world: &mut World, notification_sent: NotificationSent) {
    let NotificationKind::Speech { speaker } = notification_sent.notification.kind else {
        return;
    };
    let body = notification_sent
        .speaker
        .filter(|&body| world.get_entity(body).is_ok());
    let mut bubbles = world.resource_mut::<Bubbles>();
    let line = SpokenLine {
        notification: notification_sent.notification,
        read_by: None,
        typing: None,
    };
    match bubbles
        .speakers
        .iter_mut()
        .find(|speaking| speaking.speaker == speaker && speaking.body == body)
    {
        Some(speaking) => speaking.hear(line),
        None => {
            bubbles.arrivals += 1;
            let arrived = bubbles.arrivals;
            bubbles.speakers.push(Speaking {
                speaker,
                body,
                lines: vec![line],
                newest: None,
                replaced: 0,
                changed: Duration::ZERO,
                clock: Clock::default(),
                arrived,
                drawn: None,
            });
        }
    }
}

pub fn edge_folds(world: &World) -> Vec<u32> {
    world.resource::<Bubbles>().edge_folds.clone()
}

pub fn shown(world: &World) -> Vec<NotificationBubble> {
    world
        .resource::<Bubbles>()
        .speakers
        .iter()
        .map(|speaking| NotificationBubble {
            speaker: speaking.speaker,
            body: speaking.body,
            rect: speaking.drawn.map(|drawn| drawn.rect),
            replaced: speaking.replaced,
            lines: speaking
                .lines
                .iter()
                .map(|line| line.notification.text.plain_words())
                .collect(),
            folded: speaking.drawn.is_some_and(|drawn| drawn.folded),
        })
        .collect()
}

pub struct NotificationBubble {
    pub speaker: NpcId,
    pub body: Option<Entity>,
    pub rect: Option<egui::Rect>,
    pub replaced: u32,
    pub lines: Vec<String>,
    pub folded: bool,
}

#[derive(Resource, Default)]
struct Bubbles {
    speakers: Vec<Speaking>,
    edge_folds: Vec<u32>,
    fades: Fades<BubbleDraw>,
    arrivals: u64,
}

struct Speaking {
    speaker: NpcId,
    body: Option<Entity>,
    lines: Vec<SpokenLine>,
    newest: Option<SpokenLine>,
    replaced: u32,
    changed: Duration,
    clock: Clock,
    arrived: u64,
    drawn: Option<Drawn>,
}

#[derive(Clone, Copy)]
struct Drawn {
    rect: egui::Rect,
    folded: bool,
}

struct SpokenLine {
    notification: Notification,
    read_by: Option<Duration>,
    typing: Option<Entity>,
}

/// Kept as drawn so it can ease out after its speaker goes.
#[derive(Clone)]
struct BubbleDraw {
    key: u64,
    header: Option<String>,
    lines: Vec<LineDraw>,
    fold: Option<String>,
    tail: Option<Tail>,
    tail_offset: f32,
    rect: egui::Rect,
}

#[derive(Clone)]
struct LineDraw {
    text: StyledLine,
    shown: Option<usize>,
    read: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tail {
    Down,
    Up,
    Left,
    Right,
}

enum Spot {
    Head { crown: Vec2, feet: Vec2 },
    Off { edge: Edge, toward: Vec2 },
    Nowhere,
}

struct Room {
    bounds: Rect,
    taken: Vec<Rect>,
}

#[derive(Clone, Copy)]
struct Perch {
    tip: Vec2,
    tail: Option<Tail>,
    tail_offset: f32,
}

impl Speaking {
    fn hear(&mut self, line: SpokenLine) {
        let clock = self.clock;
        if let Some(same) = self
            .lines
            .iter_mut()
            .find(|shown| shown.notification == line.notification)
        {
            if let Some(read_by) = &mut same.read_by {
                *read_by = (*read_by).max(clock.ends_after(line.notification.stays()));
            }
            return;
        }
        if self.lines.len() < LINES {
            self.lines.push(line);
            return;
        }
        self.replaced += 1;
        self.newest = Some(line);
    }

    /// Moves the speaker's lines along, and hands back the typing of those it let go.
    fn tick(&mut self, delta: Duration, typed: &[bool]) -> Vec<Entity> {
        self.clock.advance(delta);
        let now = self.clock.now();
        let mut previous_read_by: Option<Duration> = None;
        for (line, &typed) in self.lines.iter_mut().zip(typed) {
            if line.read_by.is_none() && typed {
                let start = previous_read_by.map_or(now, |previous| previous.max(now));
                line.read_by = Some(start + Duration::from(line.notification.stays()));
            }
            previous_read_by = line.read_by;
        }
        let mut dropped = Vec::new();
        if now >= self.changed + Duration::from(REPLACE_AFTER)
            && let Some(newest) = self.newest.take()
        {
            dropped.extend(self.lines.remove(0).typing);
            self.lines.push(newest);
            self.changed = now;
        }
        let lingers = Duration::from(READ_LINGERS);
        let (kept, gone): (Vec<SpokenLine>, Vec<SpokenLine>) = std::mem::take(&mut self.lines)
            .into_iter()
            .partition(|line| line.read_by.is_none_or(|read_by| now < read_by + lingers));
        self.lines = kept;
        dropped.extend(gone.into_iter().filter_map(|line| line.typing));
        dropped
    }

    fn done(&self) -> bool {
        self.lines.is_empty() && self.newest.is_none()
    }

    /// The lines on show: each waits for the one before it to finish typing.
    fn shown_lines(&self, typed: &[bool]) -> usize {
        (1..self.lines.len())
            .find(|&index| !typed[index - 1])
            .unwrap_or(self.lines.len())
    }

    fn typing(&self) -> impl Iterator<Item = Entity> + '_ {
        self.lines.iter().filter_map(|line| line.typing)
    }
}

impl Tail {
    fn axis(self) -> egui::Vec2 {
        match self {
            Tail::Down => egui::Vec2::Y,
            Tail::Up => -egui::Vec2::Y,
            Tail::Left => -egui::Vec2::X,
            Tail::Right => egui::Vec2::X,
        }
    }

    fn turn(tail: Option<Tail>, offset: egui::Vec2) -> egui::Vec2 {
        match tail {
            Some(Tail::Down) | None => offset,
            Some(Tail::Up) => -offset,
            Some(Tail::Left) => egui::vec2(-offset.y, offset.x),
            Some(Tail::Right) => egui::vec2(offset.y, -offset.x),
        }
    }
}

impl BubbleDraw {
    fn body(&self) -> egui::Rect {
        let rect = self.rect;
        match self.tail {
            None => rect,
            Some(Tail::Down) => rect.with_max_y(rect.max.y - TAIL_OUT),
            Some(Tail::Up) => rect.with_min_y(rect.min.y + TAIL_OUT),
            Some(Tail::Left) => rect.with_min_x(rect.min.x + TAIL_OUT),
            Some(Tail::Right) => rect.with_max_x(rect.max.x - TAIL_OUT),
        }
    }
}

fn forget(world: &mut World) {
    let bubbles = std::mem::take(&mut *world.resource_mut::<Bubbles>());
    for speaking in &bubbles.speakers {
        despawn_all(world, speaking.typing());
    }
}

fn tick(world: &mut World) {
    let delta = world.resource::<Time>().delta();
    let mut bubbles = std::mem::take(&mut *world.resource_mut::<Bubbles>());
    let (gone, mut speakers): (Vec<Speaking>, Vec<Speaking>) =
        bubbles.speakers.into_iter().partition(|speaking| {
            speaking
                .body
                .is_some_and(|body| world.get_entity(body).is_err() || dead(world, body))
        });
    for speaking in &gone {
        despawn_all(world, speaking.typing());
    }
    for speaking in &mut speakers {
        let held = speaking
            .drawn
            .is_some_and(|drawn| rows::hovered(world, &[drawn.rect]));
        if held {
            continue;
        }
        let typed = typed_lines(world, speaking);
        let dropped = speaking.tick(delta, &typed);
        despawn_all(world, dropped);
    }
    speakers.retain(|speaking| !speaking.done());
    bubbles.speakers = speakers;
    *world.resource_mut::<Bubbles>() = bubbles;
}

fn place(world: &mut World) {
    let Some(gui) = gui::context(world) else {
        return;
    };
    let Some(window) = window_size(world) else {
        return;
    };
    let (now, still) = rows::motion_time(world, &gui);
    let content = world.resource::<Content>().clone();
    let mut room = Room::of(world, window);
    let viewer = session::me(world).and_then(|seen| body_at(world, seen));
    let talking_to = stage::view(world).and_then(|view| view.with);
    let mut bubbles = std::mem::take(&mut *world.resource_mut::<Bubbles>());
    let spots: Vec<Spot> = bubbles
        .speakers
        .iter()
        .map(|speaking| spot(world, speaking, window))
        .collect();
    let mut order: Vec<usize> = (0..bubbles.speakers.len()).collect();
    order.sort_by(|&a, &b| {
        let rank = |index: usize| {
            let speaking = &bubbles.speakers[index];
            let engaged = speaking
                .body
                .is_some_and(|body| Some(body) == talking_to || fighting(world, body, viewer));
            let distance = match (speaking.body.and_then(|body| body_at(world, body)), viewer) {
                (Some(at), Some(viewer)) => at.distance(viewer).0,
                _ => f32::MAX,
            };
            (!engaged, distance, speaking.arrived)
        };
        let (a, b) = (rank(a), rank(b));
        a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.cmp(&b.2))
    });
    let full = if window.x <= SMALL_WINDOW {
        FULL_IN_SMALL_WINDOW
    } else {
        FULL
    };
    let mut draws: Vec<(u64, BubbleDraw)> = Vec::new();
    let mut folded_off: Vec<(Edge, Vec2, u32)> = Vec::new();
    for (seat, &index) in order.iter().enumerate() {
        let is_full = seat < full;
        if let (Spot::Off { edge, toward }, false) = (&spots[index], is_full) {
            match folded_off.iter_mut().find(|(fold, ..)| fold == edge) {
                Some((_, _, count)) => *count += 1,
                None => folded_off.push((*edge, *toward, 1)),
            }
            bubbles.speakers[index].drawn = None;
            continue;
        }
        let speaking = &mut bubbles.speakers[index];
        start_typing(world, speaking);
        let hovered = speaking
            .drawn
            .is_some_and(|drawn| rows::hovered(world, &[drawn.rect]));
        let folded = !is_full && !hovered;
        let mut draw = speech(world, &content, speaking, folded);
        let body = body_size(&gui, world, &draw);
        let perch = match &spots[index] {
            Spot::Off { edge, toward } => Perch::docked(*edge, *toward, room.bounds),
            Spot::Head { crown, feet } => {
                Perch::over_head(*crown, *feet, framed(body, Some(Tail::Down)), room.bounds)
            }
            Spot::Nowhere => Perch::loose(room.bounds),
        };
        let size = framed(body, perch.tail);
        let perch = room.settle(perch, size);
        draw.tail = perch.tail;
        draw.tail_offset = perch.tail_offset;
        draw.rect = egui_rect(perch.rect(size));
        speaking.drawn = Some(Drawn {
            rect: draw.rect,
            folded,
        });
        draws.push((draw.key, draw));
        let babbling = is_full
            .then(|| babble_of(world, speaking))
            .flatten()
            .map(|babble| Babbling {
                babble,
                place: match speaking.body.and_then(|body| body_at(world, body)) {
                    Some(at) => SfxPlace::World(at),
                    None => SfxPlace::Interface,
                },
                rank: BabbleRank::Ranked((FULL - seat.min(FULL)) as u32),
            });
        for typing in speaking.typing().collect::<Vec<_>>() {
            match babbling {
                Some(babbling) => {
                    world.entity_mut(typing).insert(babbling);
                }
                None => {
                    world.entity_mut(typing).remove::<Babbling>();
                }
            }
        }
    }
    bubbles.edge_folds = folded_off.iter().map(|&(.., count)| count).collect();
    for &(edge, toward, count) in &folded_off {
        let mut draw = BubbleDraw {
            key: u64::MAX - edge as u64,
            header: None,
            lines: Vec::new(),
            fold: Some(fold_text(count)),
            tail: None,
            tail_offset: 0.0,
            rect: egui::Rect::NOTHING,
        };
        let perch = Perch::docked(edge, toward, room.bounds);
        let size = framed(body_size(&gui, world, &draw), perch.tail);
        let perch = room.settle(perch, size);
        draw.tail = perch.tail;
        draw.tail_offset = perch.tail_offset;
        draw.rect = egui_rect(perch.rect(size));
        draws.push((draw.key, draw));
    }
    let shown = bubbles.fades.sync(now, PRESENCE, still, draws);
    *world.resource_mut::<Bubbles>() = bubbles;
    for (draw, look) in shown {
        paint(world, &gui, &draw, look);
    }
}

fn paint(world: &mut World, gui: &egui::Context, draw: &BubbleDraw, look: Look) {
    let area = gui::world_area(world, ("bubble", draw.key))
        .fade_in(false)
        .fixed_pos(draw.rect.min);
    let shown = area.show(gui, |ui| {
        ui.multiply_opacity(look.opacity);
        let body = draw.body();
        ui.painter().rect(
            body,
            RADIUS,
            FILL.gamma_multiply(0.94),
            Stroke::new(BORDER, OUTLINE),
            StrokeKind::Inside,
        );
        let inner = body.shrink2(PADDING + egui::Vec2::splat(BORDER));
        let mut content = ui.new_child(
            UiBuilder::new()
                .max_rect(inner)
                .layout(Layout::top_down(Align::Min)),
        );
        content.spacing_mut().item_spacing.y = PART_GAP;
        if let Some(header) = &draw.header {
            content.label(
                egui::RichText::new(header)
                    .font(rows::label_font())
                    .color(INK.gamma_multiply(0.7)),
            );
        }
        if !draw.lines.is_empty() {
            content.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = LINE_GAP;
                for line in &draw.lines {
                    ui.scope(|ui| {
                        if line.read {
                            ui.multiply_opacity(READ_OPACITY);
                        }
                        let size = text::measure(
                            ui.ctx(),
                            world,
                            &line.text,
                            rows::look(LINE_SIZE, INK),
                            WIDEST,
                        );
                        let clipped = egui::vec2(size.x, size.y.min(LINES_HIGH));
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
                            rows::look(LINE_SIZE, INK),
                            line.shown,
                        );
                    });
                }
            });
        }
        if let Some(fold) = &draw.fold {
            content.label(
                egui::RichText::new(fold)
                    .font(rows::label_font())
                    .color(INK),
            );
        }
        if let Some(tail) = draw.tail {
            paint_tail(ui, body, tail, draw.tail_offset);
        }
        ui.allocate_rect(draw.rect, Sense::hover());
    });
    let look = Look {
        offset: Tail::turn(draw.tail, look.offset),
        ..look
    };
    gui.set_transform_layer(shown.response.layer_id, look.around(draw.rect));
}

fn paint_tail(ui: &egui::Ui, body: egui::Rect, tail: Tail, offset: f32) {
    let out = tail.axis();
    let along = egui::vec2(out.y.abs(), out.x.abs());
    let edge = match tail {
        Tail::Down => body.center_bottom(),
        Tail::Up => body.center_top(),
        Tail::Left => body.left_center(),
        Tail::Right => body.right_center(),
    };
    let base = edge + along * offset;
    let reach = TAIL * std::f32::consts::FRAC_1_SQRT_2;
    let (start, end, tip) = (
        base - along * reach,
        base + along * reach,
        base + out * reach,
    );
    let fill = FILL.gamma_multiply(0.94);
    ui.painter().add(Shape::convex_polygon(
        vec![start, tip, end],
        fill,
        Stroke::NONE,
    ));
    ui.painter().line_segment(
        [start + along * BORDER, end - along * BORDER],
        Stroke::new(BORDER * 2.0, fill),
    );
    let outline = Stroke::new(BORDER, OUTLINE);
    ui.painter().line_segment([start, tip], outline);
    ui.painter().line_segment([tip, end], outline);
}

fn speech(world: &World, content: &Content, speaking: &Speaking, folded: bool) -> BubbleDraw {
    let name = speaking.speaker.get(content).name.to_owned();
    let header = match speaking.replaced {
        0 => name,
        replaced => format!("{name} +{replaced}"),
    };
    let typed = typed_lines(world, speaking);
    let now = speaking.clock.now();
    let lines = speaking.lines[..speaking.shown_lines(&typed)]
        .iter()
        .map(|line| {
            let typing = line
                .typing
                .and_then(|typing| world.get::<Typewriter>(typing));
            LineDraw {
                text: line.notification.text.styled(),
                shown: typing.and_then(Typewriter::shown),
                read: line.read_by.is_some_and(|read_by| now >= read_by),
            }
        })
        .collect();
    BubbleDraw {
        key: speaking.arrived,
        header: (!folded).then_some(header),
        lines: if folded { Vec::new() } else { lines },
        fold: folded.then(|| fold_text(1)),
        tail: None,
        tail_offset: 0.0,
        rect: egui::Rect::NOTHING,
    }
}

fn fold_text(speakers: u32) -> String {
    match speakers {
        0 | 1 => "\u{2026}".to_owned(),
        speakers => format!("\u{2026} {speakers}"),
    }
}

fn body_size(gui: &egui::Context, world: &World, draw: &BubbleDraw) -> egui::Vec2 {
    let label = |text: &str| {
        gui.fonts_mut(|fonts| fonts.layout_no_wrap(text.to_owned(), rows::label_font(), INK))
            .size()
    };
    let mut parts: Vec<egui::Vec2> = Vec::new();
    parts.extend(draw.header.as_deref().map(label));
    if !draw.lines.is_empty() {
        let lines: Vec<egui::Vec2> = draw
            .lines
            .iter()
            .map(|line| {
                let size =
                    text::measure(gui, world, &line.text, rows::look(LINE_SIZE, INK), WIDEST);
                egui::vec2(size.x, size.y.min(LINES_HIGH))
            })
            .collect();
        parts.push(stacked(&lines, LINE_GAP));
    }
    parts.extend(draw.fold.as_deref().map(label));
    stacked(&parts, PART_GAP) + 2.0 * (PADDING + egui::Vec2::splat(BORDER))
}

fn stacked(parts: &[egui::Vec2], gap: f32) -> egui::Vec2 {
    let width = parts.iter().map(|part| part.x).fold(0.0, f32::max);
    let height =
        parts.iter().map(|part| part.y).sum::<f32>() + gap * parts.len().saturating_sub(1) as f32;
    egui::vec2(width, height)
}

fn framed(body: egui::Vec2, tail: Option<Tail>) -> Vec2 {
    let tail = match tail {
        Some(Tail::Down | Tail::Up) => egui::vec2(0.0, TAIL_OUT),
        Some(Tail::Left | Tail::Right) => egui::vec2(TAIL_OUT, 0.0),
        None => egui::Vec2::ZERO,
    };
    let size = body + tail;
    Vec2::new(size.x, size.y)
}

fn start_typing(world: &mut World, speaking: &mut Speaking) {
    let typed = typed_lines(world, speaking);
    let shown = speaking.shown_lines(&typed);
    for line in &mut speaking.lines[..shown] {
        if line.typing.is_none() {
            let typing = world
                .spawn((line.notification.text.styled(), Typewriter::default()))
                .id();
            line.typing = Some(typing);
        }
    }
}

fn typed_lines(world: &World, speaking: &Speaking) -> Vec<bool> {
    speaking
        .lines
        .iter()
        .map(|line| {
            line.read_by.is_some()
                || line
                    .typing
                    .and_then(|typing| world.get::<Typewriter>(typing))
                    .is_some_and(Typewriter::is_finished)
        })
        .collect()
}

fn despawn_all(world: &mut World, entities: impl IntoIterator<Item = Entity>) {
    for entity in entities.into_iter().collect::<Vec<_>>() {
        if let Ok(typing) = world.get_entity_mut(entity) {
            typing.despawn();
        }
    }
}

fn babble_of(world: &World, speaking: &Speaking) -> Option<BabbleId> {
    let content = world.resource::<Content>();
    speaking
        .body
        .and_then(|body| world.get::<Babbler>(body))
        .map(|babbler| babbler.0)
        .or(speaking.speaker.get(content).voice())
}

fn spot(world: &mut World, speaking: &Speaking, window: Vec2) -> Spot {
    let Some(body) = speaking.body else {
        return Spot::Nowhere;
    };
    let Some(at) = body_at(world, body) else {
        return Spot::Nowhere;
    };
    let (Some(crown), Some(feet)) = (
        plate::crown(world, body),
        tile_to_window(world, Pos::new(at.x, at.y + 0.5)),
    ) else {
        return Spot::Nowhere;
    };
    let edge = if feet.x < 0.0 {
        Some(Edge::Left)
    } else if feet.x > window.x {
        Some(Edge::Right)
    } else if feet.y < 0.0 {
        Some(Edge::Top)
    } else if crown.y > window.y {
        Some(Edge::Bottom)
    } else {
        None
    };
    match edge {
        Some(edge) => Spot::Off { edge, toward: feet },
        None => Spot::Head { crown, feet },
    }
}

impl Room {
    fn of(world: &World, window: Vec2) -> Room {
        let taken = hud::pieces(world)
            .into_iter()
            .chain(stage::rects(world))
            .map(|rect| Rect::new(rect.min.x, rect.min.y, rect.max.x, rect.max.y))
            .collect();
        Room {
            bounds: Rect::from_corners(Vec2::splat(EDGE), window - EDGE),
            taken,
        }
    }

    fn settle(&mut self, mut perch: Perch, size: Vec2) -> Perch {
        let reach = tail_reach(size.dot(perch.cross()));
        for _ in 0..=self.taken.len() {
            let rect = perch.rect(size);
            let Some(push) = self.taken.iter().find_map(|&taken| push_out(rect, taken)) else {
                break;
            };
            perch.nudge(push, reach);
        }
        perch.nudge(pull_in(perch.rect(size), self.bounds), reach);
        self.taken.push(perch.rect(size));
        perch
    }
}

impl Perch {
    fn over_head(crown: Vec2, feet: Vec2, size: Vec2, bounds: Rect) -> Perch {
        let above = Vec2::new(crown.x, crown.y - CROWN_GAP);
        let (tip, tail) = if above.y - size.y < bounds.min.y {
            (feet, Tail::Up)
        } else {
            (above, Tail::Down)
        };
        Perch {
            tip,
            tail: Some(tail),
            tail_offset: 0.0,
        }
    }

    fn docked(edge: Edge, toward: Vec2, bounds: Rect) -> Perch {
        let along_x = toward.x.clamp(bounds.min.x, bounds.max.x);
        let along_y = toward.y.clamp(bounds.min.y, bounds.max.y);
        let (tip, tail) = match edge {
            Edge::Left => (Vec2::new(bounds.min.x, along_y), Tail::Left),
            Edge::Right => (Vec2::new(bounds.max.x, along_y), Tail::Right),
            Edge::Top => (Vec2::new(along_x, bounds.min.y), Tail::Up),
            Edge::Bottom => (Vec2::new(along_x, bounds.max.y), Tail::Down),
        };
        Perch {
            tip,
            tail: Some(tail),
            tail_offset: 0.0,
        }
    }

    fn loose(bounds: Rect) -> Perch {
        Perch {
            tip: bounds.min,
            tail: None,
            tail_offset: 0.0,
        }
    }

    fn cross(self) -> Vec2 {
        match self.tail {
            Some(Tail::Down | Tail::Up) | None => Vec2::X,
            Some(Tail::Left | Tail::Right) => Vec2::Y,
        }
    }

    fn root(self) -> Vec2 {
        self.tip - self.cross() * self.tail_offset
    }

    fn rect(self, size: Vec2) -> Rect {
        let root = self.root();
        let min = match self.tail {
            Some(Tail::Down) => root - Vec2::new(size.x / 2.0, size.y),
            Some(Tail::Up) => root - Vec2::new(size.x / 2.0, 0.0),
            Some(Tail::Left) => root - Vec2::new(0.0, size.y / 2.0),
            Some(Tail::Right) => root - Vec2::new(size.x, size.y / 2.0),
            None => root,
        };
        Rect::from_corners(min, min + size)
    }

    fn nudge(&mut self, by: Vec2, reach: f32) {
        if self.tail.is_none() {
            self.tip += by;
            return;
        }
        let cross = self.cross();
        let along = by.dot(cross);
        let wanted = self.tail_offset - along;
        let offset = wanted.clamp(-reach, reach);
        self.tip += by - cross * along + cross * (offset - wanted);
        self.tail_offset = offset;
    }
}

fn tail_reach(body_extent: f32) -> f32 {
    (body_extent / 2.0 - RADIUS - TAIL).max(0.0)
}

fn push_out(rect: Rect, taken: Rect) -> Option<Vec2> {
    if rect.intersect(taken).is_empty() {
        return None;
    }
    let shortest = |back: f32, forth: f32| if forth < -back { forth } else { back };
    let x = shortest(taken.min.x - rect.max.x, taken.max.x - rect.min.x);
    let y = shortest(taken.min.y - rect.max.y, taken.max.y - rect.min.y);
    Some(if x.abs() < y.abs() {
        Vec2::new(x, 0.0)
    } else {
        Vec2::new(0.0, y)
    })
}

fn pull_in(rect: Rect, bounds: Rect) -> Vec2 {
    let axis = |low: f32, high: f32, min: f32, max: f32| {
        if low < min {
            min - low
        } else if high > max {
            max - high
        } else {
            0.0
        }
    };
    Vec2::new(
        axis(rect.min.x, rect.max.x, bounds.min.x, bounds.max.x),
        axis(rect.min.y, rect.max.y, bounds.min.y, bounds.max.y),
    )
}

fn egui_rect(rect: Rect) -> egui::Rect {
    egui::Rect::from_min_max(
        egui::pos2(rect.min.x.round(), rect.min.y.round()),
        egui::pos2(
            rect.min.x.round() + rect.width(),
            rect.min.y.round() + rect.height(),
        ),
    )
}

fn body_at(world: &World, body: Entity) -> Option<Pos<Tiles>> {
    world
        .get::<RenderPosition>(body)
        .map(|rendered| rendered.0)
        .or_else(|| world.get::<Position>(body).map(|position| position.pos))
}

fn dead(world: &World, body: Entity) -> bool {
    world
        .get::<Actor>(body)
        .is_some_and(|actor| actor.action == Action::Dead)
}

fn fighting(world: &World, body: Entity, viewer: Option<Pos<Tiles>>) -> bool {
    let hostile = world.get::<Attitude>(body) == Some(&Attitude::Hostile);
    let attacking = world
        .get::<Actor>(body)
        .is_some_and(|actor| actor.action == Action::Attack);
    let close = match (body_at(world, body), viewer) {
        (Some(at), Some(viewer)) => at.distance(viewer) <= FIGHTING_WITHIN,
        _ => false,
    };
    hostile && attacking && close
}

fn window_size(world: &mut World) -> Option<Vec2> {
    world
        .query_filtered::<&Window, With<PrimaryWindow>>()
        .single(world)
        .ok()
        .map(|window| Vec2::new(window.width(), window.height()))
}
