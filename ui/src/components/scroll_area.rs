use bevy_ecs::prelude::*;
use bevy_ui::{
    BorderRadius, ComputedNode, FlexDirection, Node, Overflow, PositionType, ScrollPosition, Val,
};
use bevy_ui_widgets::ScrollArea;

use crate::style::Style;
use crate::theme::color;
use crate::tokens::radius;

#[derive(Component)]
pub(crate) struct ScrollRoot;
#[derive(Component)]
pub(crate) struct ScrollViewport;
#[derive(Component)]
pub(crate) struct ScrollBar;
#[derive(Component)]
pub(crate) struct ScrollThumbMark;

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
        ScrollArea,
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
