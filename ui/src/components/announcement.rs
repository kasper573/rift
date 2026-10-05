use bevy_color::{Alpha, Color};
use bevy_ecs::hierarchy::{ChildOf, Children};
use bevy_ecs::prelude::*;
use bevy_picking::prelude::Pickable;
use bevy_scene::{CommandsSceneExt, EntityScene, Scene, bsn, template_value};
use bevy_ui::{
    AlignItems, BackgroundColor, BorderRadius, FlexDirection, JustifyContent, Node, UiRect, Val,
};

use crate::component;
use crate::components::chip::{ChipOptions, chip};
use crate::components::rich_text::{RichText, TextVoice, rich_text};
use crate::components::text::styled_text;
use crate::opacity::Opacity;
use crate::style::Style;
use crate::theme::{Family, theme};
use crate::tokens::{palette, radius, spacing, typography};

const WIDTH: f32 = 580.0;
const NEXT_OPACITY: f32 = 0.55;

#[derive(Component, Clone, Default, PartialEq)]
#[require(Node)]
pub struct AnnouncementLane {
    pub head: Option<Announcement>,
    pub next: Option<Announcement>,
    pub remaining: f32,
}

#[derive(Clone, PartialEq)]
pub struct Announcement {
    pub key: u64,
    pub speaker: Option<String>,
    pub text: RichText,
    pub kind: AnnouncementKind,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AnnouncementKind {
    #[default]
    Speech,
    Narration,
    System,
}

#[derive(Component, Default)]
pub(crate) struct Shown {
    head: Option<u64>,
    next: Option<u64>,
}

#[derive(Component, Clone, Default)]
pub(crate) struct LaneTimer;

pub fn announcement_lane(lane: AnnouncementLane) -> impl Scene {
    bsn! {
        component(lane)
        Node {
            width: Val::Px({WIDTH}),
            max_width: Val::Vw(94.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px({spacing::L}),
        }
        Pickable::IGNORE
    }
}

pub(crate) fn sync_lanes(
    lanes: Query<(Entity, &AnnouncementLane, Option<&Shown>), Changed<AnnouncementLane>>,
    mut timers: Query<&mut Node, With<LaneTimer>>,
    children: Query<&Children>,
    mut commands: Commands,
) {
    for (lane, announcements, shown) in &lanes {
        let head = announcements.head.as_ref().map(|head| head.key);
        let next = announcements.next.as_ref().map(|next| next.key);
        let current = shown.map(|shown| (shown.head, shown.next));
        if current != Some((head, next)) {
            for kid in children.get(lane).into_iter().flatten() {
                commands.entity(*kid).despawn();
            }
            if let Some(announcement) = &announcements.head {
                commands
                    .spawn_scene(card(announcement.clone(), true))
                    .insert(ChildOf(lane));
            }
            if let Some(announcement) = &announcements.next {
                commands
                    .spawn_scene(card(announcement.clone(), false))
                    .insert(ChildOf(lane));
            }
            commands.entity(lane).insert(Shown { head, next });
            continue;
        }
        let width = Val::Percent(announcements.remaining.clamp(0.0, 1.0) * 100.0);
        for timer in descendants(lane, &children) {
            if let Ok(mut node) = timers.get_mut(timer)
                && node.width != width
            {
                node.width = width;
            }
        }
    }
}

fn card(announcement: Announcement, head: bool) -> impl Scene {
    let Announcement {
        speaker,
        text,
        kind,
        ..
    } = announcement;
    let surface = theme().surface_trough;
    let ink = surface.on;
    let style = Style::new()
        .background(surface.base.with_alpha(0.92))
        .border_color(ink.with_alpha(if head { 0.55 } else { 0.3 }))
        .node(move |node| {
            node.flex_direction = FlexDirection::Column;
            node.row_gap = Val::Px(spacing::M);
            node.padding = UiRect::axes(Val::Px(spacing::XXL), Val::Px(spacing::L + 2.0));
            node.border = UiRect::all(Val::Px(if head { 2.0 } else { 1.0 }));
            node.border_radius = BorderRadius::all(Val::Px(radius::M));
        });
    let accent = match kind {
        AnnouncementKind::System => palette::AZURE_70,
        AnnouncementKind::Speech | AnnouncementKind::Narration => palette::AMBER_70,
    };
    let tag = match (head, kind) {
        (false, _) => tag_chip("WAITING", ink.with_alpha(0.6)),
        (true, AnnouncementKind::Speech) => tag_chip("ANNOUNCEMENT", accent),
        (true, AnnouncementKind::Narration) => tag_chip("NARRATION", ink.with_alpha(0.8)),
        (true, AnnouncementKind::System) => tag_chip("SYSTEM", accent),
    };
    let speaker = speaker.unwrap_or_default();
    let text = if kind == AnnouncementKind::Narration {
        RichText {
            pieces: text
                .pieces
                .into_iter()
                .map(|piece| match piece {
                    crate::RichPiece::Span(span) => span.voice(TextVoice::Whisper).into(),
                    pause => pause,
                })
                .collect(),
            ..text
        }
    } else {
        text
    };
    let text = RichText {
        size: if head { 20.0 } else { 16.0 },
        color: ink,
        ..text
    };
    let timer = head.then(|| {
        bsn! {
            Node { width: Val::Percent(100.0), height: Val::Px(3.0), margin: {UiRect::top(Val::Px(spacing::M))} }
            BackgroundColor({ink.with_alpha(0.15)})
            Children [ (
                LaneTimer
                Node { width: Val::Percent(100.0), height: Val::Percent(100.0) }
                BackgroundColor({accent})
            ) ]
        }
    });
    let justify = match kind {
        AnnouncementKind::Speech => JustifyContent::Start,
        AnnouncementKind::Narration | AnnouncementKind::System => JustifyContent::Center,
    };
    bsn! {
        template_value(style)
        component(Opacity(if head { 1.0 } else { NEXT_OPACITY }))
        Pickable::IGNORE
        Children [
            (
                Node { justify_content: JustifyContent::SpaceBetween, align_items: AlignItems::Center }
                Children [
                    {EntityScene(styled_text(speaker, ink, typography::LABEL))},
                    {EntityScene(tag)},
                ]
            ),
            (
                Node { justify_content: {justify} }
                Children [ {EntityScene(rich_text(text, head))} ]
            ),
            {timer},
        ]
    }
}

fn tag_chip(label: &str, color: Color) -> impl Scene {
    chip(ChipOptions {
        label: label.to_owned(),
        icon: None,
        family: Family {
            base: Color::NONE,
            on: color,
            hover: Color::NONE,
            active: Color::NONE,
            border: color,
        },
        link: None,
    })
}

fn descendants(root: Entity, children: &Query<&Children>) -> Vec<Entity> {
    let mut found = Vec::new();
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        if let Ok(kids) = children.get(entity) {
            stack.extend(kids.iter());
            found.extend(kids.iter());
        }
    }
    found
}
