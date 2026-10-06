use bevy_color::{Alpha, Color};
use bevy_ecs::hierarchy::{ChildOf, Children};
use bevy_ecs::prelude::*;
use bevy_math::Vec2;
use bevy_picking::hover::Hovered;
use bevy_picking::prelude::Pickable;
use bevy_scene::{CommandsSceneExt, Scene, bsn, template_value};
use bevy_ui::{
    AlignItems, BorderRadius, Display, FlexDirection, Node, Overflow, PositionType, UiRect,
    UiTransform, Val, Val2,
};

use crate::component;
use crate::components::rich_text::{RichText, rich_text};
use crate::components::text::styled_text;
use crate::cursor::ClickThrough;
use crate::motion::transition::STANDARD_EXIT;
use crate::motion::{Easing, Timing, Transform2d};
use crate::opacity::Opacity;
use crate::presence::{Leaving, Presence, PresenceMove};
use crate::style::Style;
use crate::theme::theme;
use crate::tokens::{radius, spacing, typography};

const WIDEST: f32 = 290.0;
const LINE_SIZE: f32 = 15.0;
const LINE_HEIGHT: f32 = 21.0;
const SHOWN_TEXT_LINES: f32 = 3.0;
const TAIL: f32 = 12.0;
const READ_OPACITY: f32 = 0.5;
const RISE: f32 = 10.0;
const DRIFT: f32 = 8.0;
const BUBBLE_ENTER: Timing = Timing::new(320, Easing::Standard);
const BUBBLE_EXIT: Timing = Timing::new(200, Easing::StandardAccelerate);
const LINE_PRESENCE: Presence = Presence::new(
    PresenceMove::new(
        Transform2d::new(Vec2::new(0.0, 6.0), 1.0),
        Timing::new(280, Easing::Standard),
    ),
    PresenceMove::new(Transform2d::new(Vec2::new(0.0, -6.0), 1.0), STANDARD_EXIT),
)
.collapsing();

#[derive(Component, Clone, Default, PartialEq)]
#[require(Node)]
pub struct SpeechBubble {
    pub speaker: String,
    pub replaced: u32,
    pub lines: Vec<BubbleLine>,
    pub tail: Option<BubbleTail>,
    pub tail_offset: f32,
    pub folded_speakers: Option<FoldedSpeakers>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FoldedSpeakers(pub u32);

#[derive(Clone, PartialEq)]
pub struct BubbleLine {
    pub key: u64,
    pub text: RichText,
    pub read: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BubbleTail {
    Down,
    Up,
    Left,
    Right,
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpeechLine {
    pub key: u64,
}

pub fn speech_bubble(bubble: SpeechBubble) -> impl Scene {
    let presence = bubble_presence(bubble.tail);
    bsn! {
        component(bubble)
        Node { position_type: PositionType::Absolute }
        UiTransform
        component(Hovered::default())
        ClickThrough
        Pickable { should_block_lower: false, is_hoverable: true }
        Children [
            (
                component(BubblePart::Frame)
                Node { align_items: AlignItems::Center }
                component(presence)
                Pickable { should_block_lower: false, is_hoverable: true }
                Children [
                    (
                        BubbleBody
                        template_value(body_style())
                        Pickable { should_block_lower: false, is_hoverable: true }
                        Children [
                            ( component(BubblePart::Header) Node { column_gap: Val::Px({spacing::M}) } Pickable::IGNORE ),
                            (
                                component(BubblePart::Lines)
                                Node {
                                    flex_direction: FlexDirection::Column,
                                    row_gap: Val::Px({spacing::M}),
                                    max_width: Val::Px(WIDEST),
                                }
                                Pickable::IGNORE
                            ),
                            ( component(BubblePart::Fold) Node { display: Display::None } Pickable::IGNORE ),
                        ]
                    ),
                    ( component(BubblePart::Tail) template_value(tail_style()) Pickable::IGNORE ),
                ]
            ),
        ]
    }
}

pub fn bubble_tail_reach(body_extent: f32) -> f32 {
    (body_extent / 2.0 - radius::M - TAIL).max(0.0)
}

pub fn speech_line(world: &World, bubble: Entity, key: u64) -> Option<Entity> {
    let mut stack = vec![bubble];
    while let Some(entity) = stack.pop() {
        if world
            .get::<SpeechLine>(entity)
            .is_some_and(|line| line.key == key)
        {
            return Some(entity);
        }
        if let Some(kids) = world.get::<Children>(entity) {
            stack.extend(kids.iter());
        }
    }
    None
}

#[derive(Component, Default, Clone)]
pub(crate) struct BubbleBody;

#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BubblePart {
    Frame,
    Header,
    Lines,
    Fold,
    Tail,
}

#[derive(Component, Clone, PartialEq)]
pub(crate) struct ShownBubble {
    header: (String, u32),
    folded: bool,
}

type ChangedBubble = Or<(Changed<SpeechBubble>, Changed<Hovered>)>;

#[derive(bevy_ecs::system::SystemParam)]
pub(crate) struct BubbleTree<'w, 's> {
    children: Query<'w, 's, &'static Children>,
    parts: Query<'w, 's, &'static BubblePart>,
    lines: Query<'w, 's, (Entity, &'static SpeechLine)>,
    leaving: Query<'w, 's, (), With<Leaving>>,
}

pub(crate) fn sync_bubbles(
    bubbles: Query<(Entity, &SpeechBubble, &Hovered, Option<&ShownBubble>), ChangedBubble>,
    tree: BubbleTree,
    mut nodes: Query<(&mut Node, Option<&mut UiTransform>, Option<&mut Presence>)>,
    mut commands: Commands,
) {
    for (root, bubble, hovered, shown) in &bubbles {
        let folded = bubble.folded_speakers.is_some() && !hovered.get();
        let header = (bubble.speaker.clone(), bubble.replaced);
        let next = ShownBubble {
            header: header.clone(),
            folded,
        };
        if let Ok((_, Some(mut transform), _)) = nodes.get_mut(root) {
            let anchor = anchor(bubble.tail);
            if transform.translation != anchor {
                transform.translation = anchor;
            }
        }
        let parts: Vec<(Entity, BubblePart)> = tree
            .children
            .iter_descendants(root)
            .filter_map(|part| Some((part, *tree.parts.get(part).ok()?)))
            .collect();
        for (part, kind) in parts {
            match kind {
                BubblePart::Frame => {
                    let Ok((mut node, _, Some(mut presence))) = nodes.get_mut(part) else {
                        continue;
                    };
                    node.flex_direction = match bubble.tail {
                        Some(BubbleTail::Down) | None => FlexDirection::Column,
                        Some(BubbleTail::Up) => FlexDirection::ColumnReverse,
                        Some(BubbleTail::Left) => FlexDirection::RowReverse,
                        Some(BubbleTail::Right) => FlexDirection::Row,
                    };
                    let wanted = bubble_presence(bubble.tail);
                    if (presence.enter, presence.exit) != (wanted.enter, wanted.exit) {
                        presence.enter = wanted.enter;
                        presence.exit = wanted.exit;
                    }
                }
                BubblePart::Tail => {
                    let Ok((mut node, ..)) = nodes.get_mut(part) else {
                        continue;
                    };
                    node.display = match bubble.tail {
                        Some(_) => Display::Flex,
                        None => Display::None,
                    };
                    let (left, top) = match bubble.tail {
                        Some(BubbleTail::Down | BubbleTail::Up) | None => (bubble.tail_offset, 0.0),
                        Some(BubbleTail::Left | BubbleTail::Right) => (0.0, bubble.tail_offset),
                    };
                    node.left = Val::Px(left);
                    node.top = Val::Px(top);
                    let overlap = Val::Px(-TAIL / 2.0);
                    node.margin = match bubble.tail {
                        Some(BubbleTail::Down) | None => UiRect::top(overlap),
                        Some(BubbleTail::Up) => UiRect::bottom(overlap),
                        Some(BubbleTail::Left) => UiRect::right(overlap),
                        Some(BubbleTail::Right) => UiRect::left(overlap),
                    };
                    node.border = tail_outline(bubble.tail);
                }
                BubblePart::Header => {
                    if shown.is_some_and(|shown| shown.header == header && shown.folded == folded) {
                        continue;
                    }
                    commands.entity(part).despawn_related::<Children>();
                    if !folded && !bubble.speaker.is_empty() {
                        commands
                            .spawn_scene(header_text(&bubble.speaker, bubble.replaced))
                            .insert(ChildOf(part));
                    }
                }
                BubblePart::Fold => {
                    commands.entity(part).despawn_related::<Children>();
                    if let Ok((mut node, ..)) = nodes.get_mut(part) {
                        node.display = match folded {
                            true => Display::Flex,
                            false => Display::None,
                        };
                    }
                    if folded {
                        let count = bubble
                            .folded_speakers
                            .filter(|speakers| speakers.0 > 1)
                            .map_or_else(String::new, |speakers| format!(" {}", speakers.0));
                        commands
                            .spawn_scene(styled_text(
                                format!("\u{2026}{count}"),
                                ink(),
                                typography::LABEL,
                            ))
                            .insert(ChildOf(part));
                    }
                }
                BubblePart::Lines => {
                    if let Ok((mut node, ..)) = nodes.get_mut(part) {
                        node.display = match folded {
                            true => Display::None,
                            false => Display::Flex,
                        };
                    }
                    let kept: Vec<(Entity, Entity, u64)> = tree
                        .children
                        .get(part)
                        .into_iter()
                        .flatten()
                        .filter(|&&row| !tree.leaving.contains(row))
                        .filter_map(|&row| {
                            let (text, line) = tree
                                .children
                                .get(row)
                                .into_iter()
                                .flatten()
                                .find_map(|&text| tree.lines.get(text).ok())?;
                            Some((row, text, line.key))
                        })
                        .collect();
                    for &(row, text, key) in &kept {
                        match bubble.lines.iter().find(|line| line.key == key) {
                            Some(line) => {
                                commands
                                    .entity(text)
                                    .insert(Opacity(read_opacity(line.read)));
                            }
                            None => {
                                commands.entity(row).insert(Leaving);
                            }
                        }
                    }
                    for line in &bubble.lines {
                        if kept.iter().any(|&(.., key)| key == line.key) {
                            continue;
                        }
                        commands
                            .spawn_scene(line_scene(line.clone()))
                            .insert(ChildOf(part));
                    }
                }
            }
        }
        if shown != Some(&next) {
            commands.entity(root).insert(next);
        }
    }
}

fn anchor(tail: Option<BubbleTail>) -> Val2 {
    match tail {
        Some(BubbleTail::Down) => Val2::percent(-50.0, -100.0),
        Some(BubbleTail::Up) => Val2::percent(-50.0, 0.0),
        Some(BubbleTail::Left) => Val2::percent(0.0, -50.0),
        Some(BubbleTail::Right) => Val2::percent(-100.0, -50.0),
        None => Val2::percent(0.0, 0.0),
    }
}

fn line_scene(line: BubbleLine) -> impl Scene {
    let text = RichText {
        size: LINE_SIZE,
        color: ink(),
        ..line.text
    };
    bsn! {
        Node {
            max_height: Val::Px({LINE_HEIGHT * SHOWN_TEXT_LINES}),
            overflow: Overflow::clip(),
        }
        component(LINE_PRESENCE)
        Pickable::IGNORE
        Children [
            (
                {rich_text(text, true)}
                component(SpeechLine { key: line.key })
                component(Opacity(read_opacity(line.read)))
            )
        ]
    }
}

fn read_opacity(read: bool) -> f32 {
    match read {
        true => READ_OPACITY,
        false => 1.0,
    }
}

fn bubble_presence(tail: Option<BubbleTail>) -> Presence {
    let toward = match tail {
        Some(BubbleTail::Down) | None => Vec2::Y,
        Some(BubbleTail::Up) => Vec2::NEG_Y,
        Some(BubbleTail::Left) => Vec2::NEG_X,
        Some(BubbleTail::Right) => Vec2::X,
    };
    Presence::new(
        PresenceMove::new(Transform2d::new(toward * RISE, 0.82), BUBBLE_ENTER),
        PresenceMove::new(Transform2d::new(-toward * DRIFT, 0.94), BUBBLE_EXIT),
    )
}

fn header_text(speaker: &str, replaced: u32) -> impl Scene {
    let label = match replaced {
        0 => speaker.to_owned(),
        replaced => format!("{speaker} +{replaced}"),
    };
    styled_text(label, ink().with_alpha(0.7), typography::LABEL)
}

fn tail_outline(tail: Option<BubbleTail>) -> UiRect {
    let line = Val::Px(1.0);
    match tail {
        Some(BubbleTail::Down) | None => UiRect {
            right: line,
            bottom: line,
            ..UiRect::ZERO
        },
        Some(BubbleTail::Up) => UiRect {
            left: line,
            top: line,
            ..UiRect::ZERO
        },
        Some(BubbleTail::Left) => UiRect {
            left: line,
            bottom: line,
            ..UiRect::ZERO
        },
        Some(BubbleTail::Right) => UiRect {
            top: line,
            right: line,
            ..UiRect::ZERO
        },
    }
}

fn ink() -> Color {
    theme().surface_elevated.on
}

fn body_style() -> Style {
    let surface = theme().surface_elevated;
    Style::new()
        .background(surface.base.with_alpha(0.94))
        .border_color(surface.border)
        .node(|node| {
            node.flex_direction = FlexDirection::Column;
            node.row_gap = Val::Px(spacing::S);
            node.padding = UiRect::axes(Val::Px(spacing::L + 2.0), Val::Px(spacing::L - 2.0));
            node.border = UiRect::all(Val::Px(1.0));
            node.border_radius = BorderRadius::all(Val::Px(radius::M));
        })
}

fn tail_style() -> Style {
    let surface = theme().surface_elevated;
    Style::new()
        .background(surface.base.with_alpha(0.94))
        .border_color(surface.border)
        .node(|node| {
            node.width = Val::Px(TAIL);
            node.height = Val::Px(TAIL);
        })
        .rotate(std::f32::consts::FRAC_PI_4)
}
