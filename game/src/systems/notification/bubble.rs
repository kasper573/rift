use std::time::Duration;

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use ui::{BubbleLine, BubbleTail, FoldedSpeakers, SpeechBubble};

use super::{Notification, NotificationKind, NotificationSent};
use crate::core::babble::{BabbleId, BabbleRank, Babbler, Babbling};
use crate::core::math::Pos;
use crate::core::render::tile_to_window;
use crate::core::sfx::playback::SfxPlace;
use crate::core::tiling::{TilePos, Tiles};
use crate::core::time::Seconds;
use crate::data::npc::Id as NpcId;
use crate::systems::actor::{Action, Actor, plate};
use crate::systems::combat::Attitude;
use crate::systems::dialogue::stage;
use crate::systems::hud;
use crate::systems::movement::{Position, RenderPosition};
use crate::systems::player::session::Viewpoint;
use crate::systems::scene::Scene as GameScene;

const LINES: usize = 2;
const FULL: usize = 3;
const FULL_IN_SMALL_WINDOW: usize = 2;
const SMALL_WINDOW: f32 = 960.0;
const REPLACE_AFTER: Seconds = Seconds(2.0);
const READ_LINGERS: Seconds = Seconds(0.6);
const FIGHTING_WITHIN: Tiles = Tiles(3.0);
const EDGE: f32 = ui::tokens::spacing::L;
const CROWN_GAP: f32 = ui::tokens::spacing::M;

pub struct BubblePlugin;

impl Plugin for BubblePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Bubbles>()
            .add_systems(Update, place_bubbles.run_if(in_state(GameScene::Area)))
            .add_systems(OnExit(GameScene::Area), forget);
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
    bubbles.keys += 1;
    let key = bubbles.keys;
    let line = SpokenLine {
        key,
        notification: notification_sent.notification,
        read_by: None,
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
                clock: Duration::ZERO,
                arrived,
                drawn: None,
            });
        }
    }
}

pub fn edge_folds(world: &World) -> Vec<Entity> {
    world
        .resource::<Bubbles>()
        .edge_folds
        .iter()
        .map(|(_, entity)| *entity)
        .collect()
}

pub fn shown(world: &World) -> Vec<NotificationBubble> {
    world
        .resource::<Bubbles>()
        .speakers
        .iter()
        .map(|speaking| NotificationBubble {
            speaker: speaking.speaker,
            body: speaking.body,
            drawn: speaking.drawn.map(|drawn| drawn.entity),
            replaced: speaking.replaced,
            lines: speaking
                .lines
                .iter()
                .map(|line| super::plain_words(&line.notification.text))
                .collect(),
            folded: speaking.drawn.is_some_and(|drawn| drawn.folded),
        })
        .collect()
}

pub struct NotificationBubble {
    pub speaker: NpcId,
    pub body: Option<Entity>,
    pub drawn: Option<Entity>,
    pub replaced: u32,
    pub lines: Vec<String>,
    pub folded: bool,
}

#[derive(Resource, Default)]
struct Bubbles {
    speakers: Vec<Speaking>,
    edge_folds: Vec<(Edge, Entity)>,
    keys: u64,
    arrivals: u64,
}

struct Speaking {
    speaker: NpcId,
    body: Option<Entity>,
    lines: Vec<SpokenLine>,
    newest: Option<SpokenLine>,
    replaced: u32,
    changed: Duration,
    clock: Duration,
    arrived: u64,
    drawn: Option<Drawn>,
}

#[derive(Clone, Copy)]
struct Drawn {
    entity: Entity,
    folded: bool,
}

struct SpokenLine {
    key: u64,
    notification: Notification,
    read_by: Option<Duration>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Edge {
    Left,
    Right,
    Top,
    Bottom,
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
    tail: Option<BubbleTail>,
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
                *read_by = (*read_by).max(clock + Duration::from(line.notification.stays()));
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

    fn tick(&mut self, delta: Duration, typed: &[bool]) {
        self.clock += delta;
        let now = self.clock;
        let mut previous_read_by: Option<Duration> = None;
        for (line, &typed) in self.lines.iter_mut().zip(typed) {
            if line.read_by.is_none() && typed {
                let start = previous_read_by.map_or(now, |previous| previous.max(now));
                line.read_by = Some(start + Duration::from(line.notification.stays()));
            }
            previous_read_by = line.read_by;
        }
        if now >= self.changed + Duration::from(REPLACE_AFTER)
            && let Some(newest) = self.newest.take()
        {
            self.lines.remove(0);
            self.lines.push(newest);
            self.changed = now;
        }
        let lingers = Duration::from(READ_LINGERS);
        self.lines
            .retain(|line| line.read_by.is_none_or(|read_by| now < read_by + lingers));
    }

    fn done(&self) -> bool {
        self.lines.is_empty() && self.newest.is_none()
    }

    fn shown_lines(&self, typed: &[bool]) -> Vec<BubbleLine> {
        let now = self.clock;
        let mut lines = Vec::new();
        for (index, line) in self.lines.iter().enumerate() {
            if index > 0 && !typed[index - 1] {
                break;
            }
            lines.push(BubbleLine {
                key: line.key,
                text: line.notification.text.rich(),
                read: line.read_by.is_some_and(|read_by| now >= read_by),
            });
        }
        lines
    }
}

fn forget(mut bubbles: ResMut<Bubbles>) {
    *bubbles = Bubbles::default();
}

fn place_bubbles(world: &mut World) {
    let Some(window) = window_size(world) else {
        return;
    };
    let delta = world.resource::<Time>().delta();
    let mut room = Room::of(world, window);
    let viewer = world
        .resource::<Viewpoint>()
        .0
        .and_then(|seen| body_at(world, seen));
    let talking_to = stage::view(world).and_then(|view| view.with);
    let mut bubbles = std::mem::take(&mut *world.resource_mut::<Bubbles>());
    bubbles.speakers.retain(|speaking| {
        let gone = speaking
            .body
            .is_some_and(|body| world.get_entity(body).is_err() || dead(world, body));
        if gone && let Some(drawn) = speaking.drawn {
            world.entity_mut(drawn.entity).insert(ui::Leaving);
        }
        !gone
    });
    for speaking in &mut bubbles.speakers {
        let held = speaking
            .drawn
            .and_then(|drawn| world.get::<Hovered>(drawn.entity))
            .is_some_and(Hovered::get);
        let typed = typed_lines(world, speaking);
        if !held {
            speaking.tick(delta, &typed);
        }
    }
    bubbles.speakers.retain(|speaking| {
        if speaking.done()
            && let Some(drawn) = speaking.drawn
        {
            world.entity_mut(drawn.entity).insert(ui::Leaving);
        }
        !speaking.done()
    });
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
    let full = match window.x <= SMALL_WINDOW {
        true => FULL_IN_SMALL_WINDOW,
        false => FULL,
    };
    let mut folded_off: Vec<(Edge, Vec2, u32)> = Vec::new();
    for (place, &index) in order.iter().enumerate() {
        let is_full = place < full;
        let speaking = &bubbles.speakers[index];
        let size = drawn_size(world, speaking.drawn);
        let perch = match (&spots[index], is_full) {
            (Spot::Off { edge, toward }, false) => {
                match folded_off.iter_mut().find(|(fold, ..)| fold == edge) {
                    Some((_, _, count)) => *count += 1,
                    None => folded_off.push((*edge, *toward, 1)),
                }
                if let Some(drawn) = bubbles.speakers[index].drawn.take() {
                    world.entity_mut(drawn.entity).insert(ui::Leaving);
                }
                continue;
            }
            (Spot::Off { edge, toward }, true) => Perch::docked(*edge, *toward, room.bounds),
            (Spot::Head { crown, feet }, _) => Perch::over_head(*crown, *feet, size, room.bounds),
            (Spot::Nowhere, _) => Perch::loose(room.bounds),
        };
        let perch = room.settle(perch, size);
        let typed = typed_lines(world, speaking);
        let bubble = SpeechBubble {
            speaker: speaking.speaker.get().display_name.to_owned(),
            replaced: speaking.replaced,
            lines: speaking.shown_lines(&typed),
            tail: perch.tail,
            tail_offset: perch.tail_offset,
            folded_speakers: (!is_full).then_some(FoldedSpeakers(1)),
        };
        let entity = draw(
            world,
            &mut bubbles.speakers[index],
            bubble,
            perch.root(),
            !is_full,
        );
        let babbling = match is_full {
            true => babble_of(world, &bubbles.speakers[index]).map(|babble| Babbling {
                babble,
                place: match bubbles.speakers[index]
                    .body
                    .and_then(|body| body_at(world, body))
                {
                    Some(at) => SfxPlace::World(at),
                    None => SfxPlace::Interface,
                },
                rank: BabbleRank((FULL - place.min(FULL)) as u32),
            }),
            false => None,
        };
        let keys: Vec<u64> = bubbles.speakers[index]
            .lines
            .iter()
            .map(|line| line.key)
            .collect();
        for key in keys {
            let Some(line) = ui::speech_line(world, entity, key) else {
                continue;
            };
            match babbling {
                Some(babbling) => {
                    world.entity_mut(line).insert(babbling);
                }
                None => {
                    world.entity_mut(line).remove::<Babbling>();
                }
            }
        }
    }
    draw_edge_folds(world, &mut bubbles, &folded_off, &mut room);
    *world.resource_mut::<Bubbles>() = bubbles;
}

fn draw(
    world: &mut World,
    speaking: &mut Speaking,
    bubble: SpeechBubble,
    anchor: Vec2,
    folded: bool,
) -> Entity {
    let entity = match speaking.drawn {
        Some(drawn) => {
            if let Some(mut shown) = world.get_mut::<SpeechBubble>(drawn.entity)
                && *shown != bubble
            {
                *shown = bubble;
            }
            drawn.entity
        }
        None => world
            .spawn_scene(bsn! {
                {ui::speech_bubble(bubble)}
                GlobalZIndex({ui::tokens::layer::BUBBLES})
            })
            .map(|spawned| spawned.id())
            .unwrap_or(Entity::PLACEHOLDER),
    };
    speaking.drawn = Some(Drawn { entity, folded });
    position(world, entity, anchor);
    entity
}

fn draw_edge_folds(
    world: &mut World,
    bubbles: &mut Bubbles,
    folded_off: &[(Edge, Vec2, u32)],
    room: &mut Room,
) {
    bubbles.edge_folds.retain(|(edge, entity)| {
        let keep = folded_off.iter().any(|(fold, ..)| fold == edge);
        if !keep {
            world.entity_mut(*entity).insert(ui::Leaving);
        }
        keep
    });
    for &(edge, toward, count) in folded_off {
        let drawn = bubbles
            .edge_folds
            .iter()
            .find(|(fold, _)| *fold == edge)
            .map(|&(_, entity)| entity);
        let size = drawn
            .and_then(|entity| ui::node_rect(world, entity))
            .map_or(Vec2::ZERO, |rect| rect.size());
        let perch = room.settle(Perch::docked(edge, toward, room.bounds), size);
        let bubble = SpeechBubble {
            speaker: String::new(),
            replaced: 0,
            lines: Vec::new(),
            tail: perch.tail,
            tail_offset: perch.tail_offset,
            folded_speakers: Some(FoldedSpeakers(count)),
        };
        let entity = match drawn {
            Some(entity) => {
                if let Some(mut shown) = world.get_mut::<SpeechBubble>(entity)
                    && *shown != bubble
                {
                    *shown = bubble;
                }
                entity
            }
            None => {
                let Ok(spawned) = world.spawn_scene(bsn! {
                    {ui::speech_bubble(bubble)}
                    GlobalZIndex({ui::tokens::layer::BUBBLES})
                }) else {
                    continue;
                };
                let entity = spawned.id();
                bubbles.edge_folds.push((edge, entity));
                entity
            }
        };
        position(world, entity, perch.root());
    }
}

fn position(world: &mut World, entity: Entity, anchor: Vec2) {
    let Some(mut node) = world.get_mut::<Node>(entity) else {
        return;
    };
    let (left, top) = (Val::Px(anchor.x.round()), Val::Px(anchor.y.round()));
    if node.left != left || node.top != top {
        node.left = left;
        node.top = top;
    }
}

fn typed_lines(world: &World, speaking: &Speaking) -> Vec<bool> {
    speaking
        .lines
        .iter()
        .map(|line| {
            line.read_by.is_some()
                || speaking
                    .drawn
                    .and_then(|drawn| ui::speech_line(world, drawn.entity, line.key))
                    .and_then(|text| world.get::<ui::Typewriter>(text))
                    .is_some_and(ui::Typewriter::is_finished)
        })
        .collect()
}

fn babble_of(world: &World, speaking: &Speaking) -> Option<BabbleId> {
    speaking
        .body
        .and_then(|body| world.get::<Babbler>(body))
        .map(|babbler| babbler.0)
        .or(speaking.speaker.get().babble)
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
    fn of(world: &mut World, window: Vec2) -> Room {
        let mut taken = hud::pieces(world);
        taken.extend(stage::rects(world));
        Room {
            bounds: Rect::from_corners(Vec2::splat(EDGE), window - EDGE),
            taken,
        }
    }

    fn settle(&mut self, mut perch: Perch, size: Vec2) -> Perch {
        let reach = ui::bubble_tail_reach(size.dot(perch.cross()));
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
        let (tip, tail) = match above.y - size.y < bounds.min.y {
            true => (feet, BubbleTail::Up),
            false => (above, BubbleTail::Down),
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
            Edge::Left => (Vec2::new(bounds.min.x, along_y), BubbleTail::Left),
            Edge::Right => (Vec2::new(bounds.max.x, along_y), BubbleTail::Right),
            Edge::Top => (Vec2::new(along_x, bounds.min.y), BubbleTail::Up),
            Edge::Bottom => (Vec2::new(along_x, bounds.max.y), BubbleTail::Down),
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
            Some(BubbleTail::Down | BubbleTail::Up) | None => Vec2::X,
            Some(BubbleTail::Left | BubbleTail::Right) => Vec2::Y,
        }
    }

    fn root(self) -> Vec2 {
        self.tip - self.cross() * self.tail_offset
    }

    fn rect(self, size: Vec2) -> Rect {
        let root = self.root();
        let min = match self.tail {
            Some(BubbleTail::Down) => root - Vec2::new(size.x / 2.0, size.y),
            Some(BubbleTail::Up) => root - Vec2::new(size.x / 2.0, 0.0),
            Some(BubbleTail::Left) => root - Vec2::new(0.0, size.y / 2.0),
            Some(BubbleTail::Right) => root - Vec2::new(size.x, size.y / 2.0),
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

fn push_out(rect: Rect, taken: Rect) -> Option<Vec2> {
    if rect.intersect(taken).is_empty() {
        return None;
    }
    let shortest = |back: f32, forth: f32| match forth < -back {
        true => forth,
        false => back,
    };
    let x = shortest(taken.min.x - rect.max.x, taken.max.x - rect.min.x);
    let y = shortest(taken.min.y - rect.max.y, taken.max.y - rect.min.y);
    Some(match x.abs() < y.abs() {
        true => Vec2::new(x, 0.0),
        false => Vec2::new(0.0, y),
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

fn drawn_size(world: &World, drawn: Option<Drawn>) -> Vec2 {
    drawn
        .and_then(|drawn| ui::node_rect(world, drawn.entity))
        .map_or(Vec2::ZERO, |rect| rect.size())
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
