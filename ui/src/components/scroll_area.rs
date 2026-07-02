use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::prelude::*;
use bevy_input::mouse::MouseScrollUnit;
use bevy_picking::prelude::{Drag, Pointer, Scroll};
use bevy_scene::{EntityScene, Scene, bsn, template_value};
use bevy_time::Time;
use bevy_ui::{
    BorderRadius, ComputedNode, Display, FlexDirection, Node, Overflow, PositionType,
    ScrollPosition, UiGlobalTransform, UiRect, Val,
};
use bevy_window::{PrimaryWindow, Window};

use crate::motion::transition::STANDARD_ENTER;
use crate::state::ancestor_with;
use crate::style::{StatefulPaint, Style};
use crate::theme::theme;
use crate::tokens::radius;

const SCROLL_SMOOTHING: f32 = 16.0;

#[derive(Component, Clone, Default)]
pub(crate) struct ScrollRoot;
#[derive(Component, Clone, Default)]
pub(crate) struct ScrollViewport;
#[derive(Component, Clone, Default)]
pub(crate) struct ScrollBar;
#[derive(Component, Clone, Default)]
pub(crate) struct ScrollThumbMark;

#[derive(Component, Default, Clone)]
pub(crate) struct ScrollTarget(f32);

/// Opt-in for a [`scroll_viewport`]: keeps the view glued to the bottom edge. Starts scrolled to
/// the bottom and follows it when content grows; scrolling up releases the pin, scrolling back
/// to the bottom re-engages it.
#[derive(Component, Clone)]
pub struct PinToBottom {
    pinned: bool,
    last_max: f32,
}

impl Default for PinToBottom {
    fn default() -> PinToBottom {
        PinToBottom {
            pinned: true,
            last_max: -1.0,
        }
    }
}

pub fn scrolled(content: Box<dyn Scene>) -> impl Scene {
    bsn! {
        Node { width: Val::Percent(100.0), height: Val::Percent(100.0) }
        Children [
            ( {scroll_area()}
              Children [
                ( {scroll_viewport()}
                  Children [
                    (
                        Node { width: Val::Percent(100.0), padding: {UiRect::all(Val::Px(4.0))} }
                        Children [ {EntityScene(content)} ]
                    )
                  ]
                ),
                ( {scroll_bar()} Children [ {EntityScene(scroll_thumb())} ] )
              ]
            )
        ]
    }
}

pub fn scroll_area() -> impl Scene {
    bsn! {
        ScrollRoot
        template_value(Style::new().node(|node| {
            node.flex_direction = FlexDirection::Row;
            node.overflow = Overflow::clip();
            node.position_type = PositionType::Relative;
            node.width = Val::Percent(100.0);
        }))
    }
}

pub fn scroll_viewport() -> impl Scene {
    bsn! {
        ScrollViewport
        ScrollPosition::default()
        ScrollTarget::default()
        template_value(Style::new().node(|node| {
            node.overflow = Overflow::scroll_y();
            node.flex_grow = 1.0;
            node.height = Val::Percent(100.0);
            node.flex_direction = FlexDirection::Column;
        }))
    }
}

pub fn scroll_bar() -> impl Scene {
    bsn! {
        ScrollBar
        template_value(Style::new()
            .background(theme().surface_canvas.hover)
            .node(|node| {
                node.width = Val::Px(10.0);
                node.height = Val::Percent(100.0);
                node.position_type = PositionType::Relative;
                node.border_radius = BorderRadius::all(Val::Px(radius::PILL));
            }))
    }
}

pub fn scroll_thumb() -> impl Scene {
    bsn! {
        ScrollThumbMark
        template_value(Style::new()
            .background(
                StatefulPaint::new(theme().surface_canvas.border)
                    .hover(theme().surface_canvas.on)
                    .active(theme().surface_canvas.on),
            )
            .transition(STANDARD_ENTER)
            .node(|node| {
                node.position_type = PositionType::Absolute;
                node.left = Val::Px(2.0);
                node.width = Val::Px(6.0);
                node.border_radius = BorderRadius::all(Val::Px(radius::PILL));
            }))
    }
}

pub fn scroll_corner() -> impl Scene {
    bsn! { Node }
}

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

pub(crate) fn pin_to_bottom(
    mut viewports: Query<
        (&ComputedNode, &mut ScrollTarget, &mut PinToBottom),
        With<ScrollViewport>,
    >,
) {
    const EPS: f32 = 1.0;
    for (view, mut target, mut pin) in &mut viewports {
        let scale = view.inverse_scale_factor;
        let max = (view.content_size.y * scale - view.size.y * scale).max(0.0);
        if (max - pin.last_max).abs() > EPS {
            pin.last_max = max;
            if pin.pinned {
                target.0 = max;
            }
        } else {
            pin.pinned = target.0 >= max - EPS;
        }
    }
}

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

#[allow(clippy::type_complexity)]
pub(crate) fn sync_scrollbars(
    roots: Query<&Children, With<ScrollRoot>>,
    is_viewport: Query<(), With<ScrollViewport>>,
    is_bar: Query<(), With<ScrollBar>>,
    viewports: Query<&ComputedNode, With<ScrollViewport>>,
    mut bars: Query<
        (&ComputedNode, &Children, &mut Node),
        (With<ScrollBar>, Without<ScrollThumbMark>),
    >,
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
        let (Ok(view), Ok((bar_node, bar_children, mut bar_style))) =
            (viewports.get(viewport), bars.get_mut(bar))
        else {
            continue;
        };
        let scale = view.inverse_scale_factor;
        let visible = view.size.y * scale;
        let content = view.content_size.y * scale;

        bar_style.display = if content > visible + 0.5 {
            Display::Flex
        } else {
            Display::None
        };

        let Some(thumb) = bar_children.iter().next() else {
            continue;
        };
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
