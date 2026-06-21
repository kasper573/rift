use std::time::Duration;

use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::prelude::*;
use bevy_math::Vec2;
use bevy_picking::prelude::{Click, Out, Over, Pointer};
use bevy_time::Time;
use bevy_ui::{BorderRadius, FlexDirection, Node, PositionType, UiRect, UiTransform, Val};

use crate::motion::transition::STANDARD_ENTER;
use crate::motion::{Motion, Transform2d};
use crate::state::ancestor_with;
use crate::style::Style;
use crate::theme::color;
use crate::tokens::{radius, spacing};
use bevy_opacity::Opacity;

const CARD_WIDTH: f32 = 356.0;
const CARD_HEIGHT: f32 = 76.0;
const TRAVEL: f32 = 100.0;
const PEEK: f32 = 16.0;
const PEEK_SCALE: f32 = 0.05;
const GAP: f32 = 14.0;
const MAX_VISIBLE: usize = 3;
const STACK_HEIGHT: f32 = MAX_VISIBLE as f32 * (CARD_HEIGHT + GAP);
const EDGE: f32 = 24.0;
const TOAST_TTL: Duration = Duration::from_secs(4);

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum SonnerPosition {
    #[default]
    BottomRight,
}

impl SonnerPosition {
    fn grow(self) -> Vec2 {
        Vec2::new(0.0, -1.0)
    }

    fn travel(self) -> Vec2 {
        Vec2::new(0.0, 1.0)
    }
}

#[derive(Component)]
#[require(Node)]
pub struct Toaster {
    pub position: SonnerPosition,
    pub expanded: bool,
}

#[derive(Component)]
#[require(Node)]
pub struct Toast {
    pub leaving: bool,
    age: Duration,
}

#[derive(Component)]
pub struct ToastLeaving(Duration);

#[derive(Component)]
#[require(Node)]
pub struct ToastClose;

pub fn toaster(position: SonnerPosition) -> impl Bundle {
    (
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(EDGE),
            right: Val::Px(EDGE),
            width: Val::Px(CARD_WIDTH),
            height: Val::Px(STACK_HEIGHT),
            ..Node::default()
        },
        Toaster {
            position,
            expanded: false,
        },
    )
}

pub fn toast() -> impl Bundle {
    (
        card_node(),
        Toast {
            leaving: false,
            age: Duration::ZERO,
        },
        Motion::default(),
        Opacity::new(0.0),
        UiTransform::default(),
        card_style(),
    )
}

pub fn sonner_close() -> impl Bundle {
    (Node::default(), ToastClose)
}

pub(crate) fn on_close(
    click: On<Pointer<Click>>,
    is_close: Query<(), With<ToastClose>>,
    parents: Query<&ChildOf>,
    has_toast: Query<(), With<Toast>>,
    mut toasts: Query<&mut Toast>,
) {
    if ancestor_with::<ToastClose>(click.entity, &parents, &is_close).is_none() {
        return;
    }
    if let Some(toast) = ancestor_with::<Toast>(click.entity, &parents, &has_toast)
        && let Ok(mut toast) = toasts.get_mut(toast)
    {
        toast.leaving = true;
    }
}

pub(crate) fn age_toasts(
    time: Res<Time>,
    toasters: Query<(&Toaster, &Children)>,
    mut toasts: Query<&mut Toast>,
) {
    let dt = time.delta();
    for (toaster, children) in &toasters {
        if toaster.expanded {
            continue;
        }
        for &child in children {
            let Ok(mut toast) = toasts.get_mut(child) else {
                continue;
            };
            if toast.leaving {
                continue;
            }
            toast.age += dt;
            if toast.age >= TOAST_TTL {
                toast.leaving = true;
            }
        }
    }
}

pub(crate) fn size_toaster(
    mut toasters: Query<(&Toaster, &Children, &mut Node)>,
    toasts: Query<&Toast>,
) {
    for (toaster, children, mut node) in &mut toasters {
        let height = if toaster.expanded {
            let live = children
                .iter()
                .filter(|&child| toasts.get(child).is_ok_and(|toast| !toast.leaving))
                .count();
            live.max(MAX_VISIBLE) as f32 * (CARD_HEIGHT + GAP)
        } else {
            STACK_HEIGHT
        };
        node.height = Val::Px(height);
    }
}

pub(crate) fn toaster_hover(
    over: On<Pointer<Over>>,
    parents: Query<&ChildOf>,
    is_toaster: Query<(), With<Toaster>>,
    mut toasters: Query<&mut Toaster>,
) {
    if let Some(region) = ancestor_with::<Toaster>(over.entity, &parents, &is_toaster)
        && let Ok(mut toaster) = toasters.get_mut(region)
    {
        toaster.expanded = true;
    }
}

pub(crate) fn toaster_leave(
    out: On<Pointer<Out>>,
    parents: Query<&ChildOf>,
    is_toaster: Query<(), With<Toaster>>,
    mut toasters: Query<&mut Toaster>,
) {
    if let Some(region) = ancestor_with::<Toaster>(out.entity, &parents, &is_toaster)
        && let Ok(mut toaster) = toasters.get_mut(region)
    {
        toaster.expanded = false;
    }
}

pub(crate) fn layout_toasts(
    toasters: Query<(&Toaster, &Children)>,
    mut cards: Query<(&Toast, &mut Node, &mut Motion)>,
) {
    for (toaster, children) in &toasters {
        let mut depth = 0usize;
        for &entity in children.iter().rev().collect::<Vec<_>>().iter() {
            let Ok((toast, mut node, mut motion)) = cards.get_mut(entity) else {
                continue;
            };
            let here = depth;
            if !toast.leaving {
                depth += 1;
            }
            let visible = here < MAX_VISIBLE || toast.leaving || toaster.expanded;
            node.display = if visible {
                bevy_ui::Display::Flex
            } else {
                bevy_ui::Display::None
            };

            let grow = toaster.position.grow();
            let off = toaster.position.travel() * TRAVEL;
            let here = here as f32;
            let rest = if toaster.expanded {
                grow * here * (CARD_HEIGHT + GAP)
            } else {
                grow * here * PEEK
            };
            let scale = if toaster.expanded {
                1.0
            } else {
                1.0 - here * PEEK_SCALE
            };
            let (translate, scale, opacity) = if toast.leaving {
                (rest, scale, 0.0)
            } else {
                (rest, scale, 1.0)
            };
            let enter = Transform2d {
                translation: rest + off,
                scale: Vec2::splat(0.9),
                rotation: 0.0,
            };
            let target = Transform2d {
                translation: translate,
                scale: Vec2::splat(scale),
                rotation: 0.0,
            };
            motion.aim_transform(enter, target, Some(STANDARD_ENTER));
            motion.aim_opacity(0.0, opacity, Some(STANDARD_ENTER));
        }
    }
}

pub(crate) fn reap_toasts(
    time: Res<Time>,
    mut commands: Commands,
    leaving: Query<(Entity, &Toast, Option<&ToastLeaving>)>,
) {
    let now = time.elapsed();
    for (entity, toast, marker) in &leaving {
        match (toast.leaving, marker) {
            (true, None) => {
                commands.entity(entity).insert(ToastLeaving(now));
            }
            (true, Some(ToastLeaving(since)))
                if now.saturating_sub(*since) >= STANDARD_ENTER.duration =>
            {
                commands.entity(entity).despawn();
            }
            _ => {}
        }
    }
}

fn card_node() -> Node {
    Node {
        position_type: PositionType::Absolute,
        bottom: Val::Px(0.0),
        right: Val::Px(0.0),
        width: Val::Px(CARD_WIDTH),
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(spacing::S),
        padding: UiRect::all(Val::Px(spacing::XL)),
        border: UiRect::all(Val::Px(1.0)),
        border_radius: BorderRadius::all(Val::Px(radius::M)),
        ..Node::default()
    }
}

fn card_style() -> Style {
    Style::new()
        .background(color::surface_elevated.base)
        .border_color(color::surface_elevated.border)
}
