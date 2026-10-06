use std::time::Duration;

use bevy_color::Alpha;
use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::prelude::*;
use bevy_math::Vec2;
use bevy_picking::prelude::{Click, Out, Over, Pickable, Pointer};
use bevy_scene::{Scene, bsn, template_value};
use bevy_time::Time;
use bevy_ui::{
    AlignItems, BorderRadius, FlexDirection, Node, PositionType, UiRect, UiTransform, Val,
};

use crate::component;
use crate::components::rich_text::MotionPreference;
use crate::cursor::{ClickThrough, CursorStyle};
use crate::motion::transition::{STANDARD_ENTER, STANDARD_EXIT};
use crate::motion::{Motion, Transform2d};
use crate::opacity::Opacity;
use crate::state::ancestor_with;
use crate::style::Style;
use crate::theme::theme;
use crate::tokens::{radius, spacing};

const CARD_WIDTH: f32 = 356.0;
const CARD_HEIGHT: f32 = 76.0;
const TRAVEL: f32 = 100.0;
const PEEK: f32 = 16.0;
const PEEK_SCALE: f32 = 0.05;
const GAP: f32 = 14.0;
const MAX_VISIBLE: usize = 3;
const EDGE: f32 = 24.0;
const TOAST_TTL: Duration = Duration::from_secs(4);
const COMPACT_GAP: f32 = 4.0;
const COMPACT_WIDTH: f32 = 320.0;
const COMPACT_SHOWN: usize = 3;
const COMPACT_HELD: usize = 8;
const COMPACT_SLIDE: f32 = 24.0;
const BUMP_SCALE: f32 = 1.08;
const LEAVE_TRAVEL: f32 = 0.3;
const LEAVE_SCALE: f32 = 0.94;
const COMPACT_LEAVE_SLIDE: f32 = 0.75;
const COMPACT_LEAVE_SCALE: f32 = 0.96;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum SonnerLook {
    #[default]
    Card,
    Compact,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum SonnerPosition {
    #[default]
    BottomRight,
    TopCenter,
}

impl SonnerPosition {
    fn grow(self) -> Vec2 {
        match self {
            SonnerPosition::BottomRight => Vec2::new(0.0, -1.0),
            SonnerPosition::TopCenter => Vec2::new(0.0, 1.0),
        }
    }

    fn travel(self) -> Vec2 {
        -self.grow()
    }

    fn place_toaster(self, node: &mut Node) {
        match self {
            SonnerPosition::BottomRight => {
                node.bottom = Val::Px(EDGE);
                node.right = Val::Px(EDGE);
            }
            SonnerPosition::TopCenter => {
                node.top = Val::Px(EDGE);
                node.left = Val::Percent(50.0);
                node.margin = UiRect::left(Val::Px(-CARD_WIDTH / 2.0));
            }
        }
    }

    fn place_card(self, node: &mut Node) {
        match self {
            SonnerPosition::BottomRight => {
                node.bottom = Val::Px(0.0);
                node.top = Val::Auto;
            }
            SonnerPosition::TopCenter => {
                node.top = Val::Px(0.0);
                node.bottom = Val::Auto;
            }
        }
    }
}

#[derive(Component, Clone, Default)]
#[require(Node)]
pub struct Toaster {
    pub position: SonnerPosition,
    pub expanded: bool,
    pub look: SonnerLook,
    pub overflow: usize,
}

#[derive(Component, Clone, Default)]
#[require(Node)]
pub struct Toast {
    pub leaving: bool,
    age: Duration,
    lasts: Option<Duration>,
    bumped: bool,
}

#[derive(Component, Clone)]
pub struct ToastLeaving(Duration);

#[derive(Component, Clone, Default)]
#[require(Node, CursorStyle::Pointer)]
pub struct ToastClose;

pub fn toaster(position: SonnerPosition) -> impl Scene {
    let mut node = Node {
        position_type: PositionType::Absolute,
        width: Val::Px(CARD_WIDTH),
        height: Val::Px(0.0),
        ..Node::default()
    };
    position.place_toaster(&mut node);
    bsn! {
        template_value(node)
        Toaster { position: {position} }
    }
}

pub fn toast(lasts: Duration) -> impl Scene {
    bsn! {
        component(card_node())
        Toast { lasts: {Some(lasts)} }
        Motion
        component(Opacity(0.0))
        UiTransform
        template_value(card_style())
    }
}

pub fn compact_toaster(max_width: f32) -> impl Scene {
    let node = Node {
        position_type: PositionType::Absolute,
        bottom: Val::Px(EDGE),
        right: Val::Px(EDGE),
        max_width: Val::Px(max_width.min(COMPACT_WIDTH)),
        flex_direction: FlexDirection::Column,
        align_items: AlignItems::FlexEnd,
        row_gap: Val::Px(COMPACT_GAP),
        ..Node::default()
    };
    bsn! {
        template_value(node)
        Toaster { position: {SonnerPosition::BottomRight}, look: {SonnerLook::Compact} }
        Pickable::IGNORE
    }
}

pub fn compact_toast(lasts: Duration) -> impl Scene {
    bsn! {
        component(compact_node())
        Toast { lasts: {Some(lasts)} }
        Motion
        component(Opacity(0.0))
        UiTransform
        template_value(compact_style())
        ClickThrough
        Pickable { should_block_lower: false, is_hoverable: true }
    }
}

pub fn bump_toast(world: &mut World, toast: Entity) {
    let Some(mut shown) = world.get_mut::<Toast>(toast) else {
        return;
    };
    shown.age = Duration::ZERO;
    shown.bumped = true;
    if let Some(toaster) = world.get::<ChildOf>(toast).map(ChildOf::parent) {
        world.entity_mut(toaster).detach_child(toast);
        world.entity_mut(toaster).add_child(toast);
    }
}

pub fn sonner_close() -> impl Scene {
    bsn! {
        ToastClose
    }
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
            if toast.age >= toast.lasts.unwrap_or(TOAST_TTL) {
                toast.leaving = true;
            }
        }
    }
}

pub(crate) fn size_toaster(
    mut toasters: Query<(&Toaster, Option<&Children>, &mut Node)>,
    toasts: Query<&Toast>,
) {
    for (toaster, children, mut node) in &mut toasters {
        if toaster.look == SonnerLook::Compact {
            continue;
        }
        let held = children
            .into_iter()
            .flatten()
            .filter(|&&child| toasts.contains(child))
            .count();
        let live = children
            .into_iter()
            .flatten()
            .filter(|&&child| toasts.get(child).is_ok_and(|toast| !toast.leaving))
            .count();
        let height = match (held, toaster.expanded) {
            (0, _) => 0.0,
            (_, true) => live.max(1) as f32 * (CARD_HEIGHT + GAP),
            (_, false) => CARD_HEIGHT + (held.min(MAX_VISIBLE) - 1) as f32 * PEEK,
        };
        let height = Val::Px(height);
        if node.height != height {
            node.height = height;
        }
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
    mut toasters: Query<(&mut Toaster, &Children)>,
    mut cards: Query<(&mut Toast, &mut Node, &mut Motion)>,
    preference: Res<MotionPreference>,
) {
    for (mut toaster, children) in &mut toasters {
        if toaster.look == SonnerLook::Compact {
            let overflow = layout_compact(&toaster, children, &mut cards, preference.reduced);
            if toaster.overflow != overflow {
                toaster.overflow = overflow;
            }
            continue;
        }
        let mut depth = 0usize;
        for &entity in children.iter().rev().collect::<Vec<_>>().iter() {
            let Ok((toast, mut node, mut motion)) = cards.get_mut(entity) else {
                continue;
            };
            let toast = &*toast;
            toaster.position.place_card(&mut node);
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
            let enter = Transform2d::new(rest + off, 0.9);
            let (target, opacity, timing) = match toast.leaving {
                true => (
                    Transform2d::new(rest + off * LEAVE_TRAVEL, scale * LEAVE_SCALE),
                    0.0,
                    STANDARD_EXIT,
                ),
                false => (Transform2d::new(rest, scale), 1.0, STANDARD_ENTER),
            };
            motion.aim_transform(enter, target, Some(timing));
            motion.aim_opacity(0.0, opacity, Some(timing));
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
                if now.saturating_sub(*since) >= STANDARD_EXIT.duration =>
            {
                commands.entity(entity).despawn();
            }
            _ => {}
        }
    }
}

fn layout_compact(
    toaster: &Toaster,
    children: &Children,
    cards: &mut Query<(&mut Toast, &mut Node, &mut Motion)>,
    reduced: bool,
) -> usize {
    let limit = match toaster.expanded {
        true => COMPACT_HELD,
        false => COMPACT_SHOWN,
    };
    let mut live = 0usize;
    for &entity in children.iter().rev().collect::<Vec<_>>().iter() {
        let Ok((mut toast, mut node, mut motion)) = cards.get_mut(entity) else {
            continue;
        };
        let here = live;
        if !toast.leaving {
            live += 1;
        }
        let display = match here < limit || toast.leaving {
            true => bevy_ui::Display::Flex,
            false => bevy_ui::Display::None,
        };
        if node.display != display {
            node.display = display;
        }
        let slide = match reduced {
            true => Vec2::ZERO,
            false => Vec2::new(COMPACT_SLIDE, 0.0),
        };
        let enter = Transform2d::new(slide, 1.0);
        let rest = Transform2d::IDENTITY;
        if std::mem::take(&mut toast.bumped) && !reduced {
            let swell = Transform2d::new(Vec2::ZERO, BUMP_SCALE);
            motion.aim_transform(swell, swell, None);
        }
        let gone = match reduced {
            true => rest,
            false => Transform2d::new(slide * COMPACT_LEAVE_SLIDE, COMPACT_LEAVE_SCALE),
        };
        let (target, opacity, timing) = match toast.leaving {
            true => (gone, 0.0, STANDARD_EXIT),
            false => (rest, 1.0, STANDARD_ENTER),
        };
        motion.aim_transform(enter, target, Some(timing));
        motion.aim_opacity(0.0, opacity, Some(timing));
    }
    live.saturating_sub(limit)
}

fn compact_node() -> Node {
    Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        column_gap: Val::Px(spacing::M),
        padding: UiRect::axes(Val::Px(spacing::L), Val::Px(spacing::S)),
        border: UiRect::all(Val::Px(1.0)),
        border_radius: BorderRadius::all(Val::Px(radius::M)),
        ..Node::default()
    }
}

fn compact_style() -> Style {
    Style::new()
        .background(theme().surface_elevated.base.with_alpha(0.72))
        .border_color(theme().surface_elevated.border.with_alpha(0.6))
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
        .background(theme().surface_elevated.base)
        .border_color(theme().surface_elevated.border)
}
