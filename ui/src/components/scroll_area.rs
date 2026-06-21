use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::prelude::*;
use bevy_input::mouse::MouseScrollUnit;
use bevy_picking::prelude::{Drag, Pointer, Scroll};
use bevy_time::Time;
use bevy_ui::{
    BorderRadius, ComputedNode, FlexDirection, Node, Overflow, PositionType, ScrollPosition,
    UiGlobalTransform, Val,
};
use bevy_window::{PrimaryWindow, Window};

use crate::motion::transition::STANDARD_ENTER;
use crate::state::ancestor_with;
use crate::style::Style;
use crate::theme::color;
use crate::tokens::radius;

// Exponential approach rate: higher snaps faster. Frame-rate independent via `dt`.
const SCROLL_SMOOTHING: f32 = 16.0;

#[derive(Component)]
pub(crate) struct ScrollRoot;
#[derive(Component)]
pub(crate) struct ScrollViewport;
#[derive(Component)]
pub(crate) struct ScrollBar;
#[derive(Component)]
pub(crate) struct ScrollThumbMark;

// The scroll offset the viewport is heading toward; `ScrollPosition` eases to it so wheel and drag
// scrolling animate instead of snapping.
#[derive(Component, Default)]
pub(crate) struct ScrollTarget(f32);

pub fn scroll_area() -> impl Bundle {
    (
        Node::default(),
        ScrollRoot,
        Style::new().node(|node| {
            node.flex_direction = FlexDirection::Row;
            node.overflow = Overflow::clip();
            node.position_type = PositionType::Relative;
        }),
    )
}

pub fn scroll_viewport() -> impl Bundle {
    (
        Node::default(),
        ScrollViewport,
        ScrollPosition::default(),
        ScrollTarget::default(),
        Style::new().node(|node| {
            node.overflow = Overflow::scroll_y();
            node.flex_grow = 1.0;
            node.height = Val::Percent(100.0);
            node.flex_direction = FlexDirection::Column;
        }),
    )
}

pub fn scroll_bar() -> impl Bundle {
    (
        Node::default(),
        ScrollBar,
        Style::new()
            .background(color::surface_canvas_hover)
            .node(|node| {
                node.width = Val::Px(10.0);
                node.height = Val::Percent(100.0);
                node.position_type = PositionType::Relative;
                node.border_radius = BorderRadius::all(Val::Px(radius::PILL));
            }),
    )
}

pub fn scroll_thumb() -> impl Bundle {
    (
        Node::default(),
        ScrollThumbMark,
        Style::new()
            .background(color::surface_canvas_border)
            .hover(Style::new().background(color::surface_canvas_on_soft))
            .active(Style::new().background(color::surface_canvas_on))
            .transition(STANDARD_ENTER)
            .node(|node| {
                node.position_type = PositionType::Absolute;
                node.left = Val::Px(2.0);
                node.width = Val::Px(6.0);
                node.border_radius = BorderRadius::all(Val::Px(radius::PILL));
            }),
    )
}

pub fn scroll_corner() -> impl Bundle {
    Node::default()
}

// Wheel/trackpad over a viewport nudges its target; `animate_scroll` carries the real offset there.
pub(crate) fn on_scroll(
    mut scroll: On<Pointer<Scroll>>,
    mut viewports: Query<(&ComputedNode, &mut ScrollTarget), With<ScrollViewport>>,
) {
    let Ok((view, mut target)) = viewports.get_mut(scroll.entity) else {
        return;
    };
    scroll.propagate(false);
    let scale = view.inverse_scale_factor;
    let max = (view.content_size.y * scale - view.size.y * scale).max(0.0);
    let delta = scroll.y
        * match scroll.unit {
            MouseScrollUnit::Line => MouseScrollUnit::SCROLL_UNIT_CONVERSION_FACTOR,
            MouseScrollUnit::Pixel => 1.0,
        };
    target.0 = (target.0 - delta).clamp(0.0, max);
}

pub(crate) fn animate_scroll(
    time: Res<Time>,
    mut viewports: Query<(&mut ScrollPosition, &ScrollTarget), With<ScrollViewport>>,
) {
    let factor = 1.0 - (-time.delta_secs() * SCROLL_SMOOTHING).exp();
    for (mut position, target) in &mut viewports {
        let delta = target.0 - position.y;
        position.y = if delta.abs() < 0.5 {
            target.0
        } else {
            position.y + delta * factor
        };
    }
}

// Dragging the thumb maps the cursor's position within the bar straight onto the scroll offset, and
// sets the live position too so the thumb tracks the cursor without the easing lag.
#[allow(clippy::too_many_arguments)]
pub(crate) fn on_thumb_drag(
    drag: On<Pointer<Drag>>,
    thumbs: Query<(), With<ScrollThumbMark>>,
    parents: Query<&ChildOf>,
    is_root: Query<(), With<ScrollRoot>>,
    roots: Query<&Children, With<ScrollRoot>>,
    is_viewport: Query<(), With<ScrollViewport>>,
    measured: Query<(&ComputedNode, &UiGlobalTransform)>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut scroll: Query<(&mut ScrollTarget, &mut ScrollPosition)>,
) {
    let thumb = drag.entity;
    if !thumbs.contains(thumb) {
        return;
    }
    let (Ok(bar), Some(root)) = (
        parents.get(thumb).map(ChildOf::parent),
        ancestor_with::<ScrollRoot>(thumb, &parents, &is_root),
    ) else {
        return;
    };
    let Ok(children) = roots.get(root) else {
        return;
    };
    let Some(viewport) = children
        .iter()
        .find(|&child| is_viewport.get(child).is_ok())
    else {
        return;
    };
    let (Ok((bar_node, bar_transform)), Ok((view, _))) =
        (measured.get(bar), measured.get(viewport))
    else {
        return;
    };
    let bar_scale = bar_node.inverse_scale_factor;
    let height = bar_node.size.y * bar_scale;
    let top = bar_transform.translation.y * bar_scale - height / 2.0;
    let view_scale = view.inverse_scale_factor;
    let max = (view.content_size.y * view_scale - view.size.y * view_scale).max(0.0);
    let Some(cursor) = windows.iter().next().and_then(Window::cursor_position) else {
        return;
    };
    if height <= 0.0 {
        return;
    }
    let fraction = ((cursor.y - top) / height).clamp(0.0, 1.0);
    if let Ok((mut target, mut position)) = scroll.get_mut(viewport) {
        target.0 = fraction * max;
        position.y = target.0;
    }
}

/// Sizes and positions thumbs from viewports. Runs after layout so measured sizes are current.
pub(crate) fn sync_scrollbars(
    roots: Query<&Children, With<ScrollRoot>>,
    is_viewport: Query<(), With<ScrollViewport>>,
    is_bar: Query<(), With<ScrollBar>>,
    viewports: Query<&ComputedNode, With<ScrollViewport>>,
    bars: Query<(&ComputedNode, &Children), With<ScrollBar>>,
    mut thumbs: Query<&mut Node, With<ScrollThumbMark>>,
) {
    for children in &roots {
        let viewport = children
            .iter()
            .find(|&child| is_viewport.get(child).is_ok());
        let bar = children.iter().find(|&child| is_bar.get(child).is_ok());
        let (Some(viewport), Some(bar)) = (viewport, bar) else {
            continue;
        };
        let (Ok(view), Ok((bar_node, bar_children))) = (viewports.get(viewport), bars.get(bar))
        else {
            continue;
        };
        let Some(thumb) = bar_children.iter().next() else {
            continue;
        };
        let scale = view.inverse_scale_factor;
        let visible = view.size.y * scale;
        let content = view.content_size.y * scale;
        let scrolled = view.scroll_position.y * scale;
        let track = bar_node.size.y * bar_node.inverse_scale_factor;

        let ratio = if content > 0.0 {
            (visible / content).min(1.0)
        } else {
            1.0
        };
        let thumb_len = (track * ratio).max(24.0).min(track);
        let max_scroll = (content - visible).max(0.0);
        let fraction = if max_scroll > 0.0 {
            (scrolled / max_scroll).clamp(0.0, 1.0)
        } else {
            0.0
        };
        if let Ok(mut node) = thumbs.get_mut(thumb) {
            node.height = Val::Px(thumb_len);
            node.top = Val::Px((track - thumb_len).max(0.0) * fraction);
        }
    }
}
