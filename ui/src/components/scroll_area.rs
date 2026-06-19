//! `ScrollArea`: a fixed-size region whose viewport clips and scrolls its overflowing content (via the
//! `Node`'s overflow and a [`ScrollPosition`]), paired with a scrollbar whose thumb is sized and moved
//! to mirror how much is shown and where — kept in sync by [`sync_scrollbars`] from the viewport's
//! measured content. The app owns the scroll offset; the showcase animates it.

use bevy_ecs::prelude::*;
use bevy_ui::{
    BorderRadius, ComputedNode, FlexDirection, Node, Overflow, PositionType, ScrollPosition, Val,
};
use bevy_view::{View, node};

use crate::recipe::{Style, Styled};
use crate::theme::color;
use crate::tokens::radius;

/// Marks a scroll area's root, viewport, scrollbar and thumb so [`sync_scrollbars`] can pair them up.
#[derive(Component, Clone)]
pub(crate) struct ScrollRoot;
#[derive(Component, Clone)]
pub(crate) struct ScrollViewport;
#[derive(Component, Clone)]
pub(crate) struct ScrollBar;
#[derive(Component, Clone)]
pub(crate) struct ScrollThumbMark;

#[derive(Default)]
pub struct ScrollArea {
    children: Vec<View>,
}

children_builder!(ScrollArea);

/// The clipping, scrolling viewport.
#[derive(Default)]
pub struct ScrollAreaViewport {
    children: Vec<View>,
}

children_builder!(ScrollAreaViewport);

/// A scrollbar track.
#[derive(Default)]
pub struct ScrollAreaScrollbar {
    children: Vec<View>,
}

children_builder!(ScrollAreaScrollbar);

/// The thumb within a scrollbar; its length and position track the viewport.
#[derive(Default)]
pub struct ScrollAreaThumb;

/// The corner where two scrollbars meet.
#[derive(Default)]
pub struct ScrollAreaCorner;

impl From<ScrollArea> for View {
    fn from(area: ScrollArea) -> View {
        node()
            .insert(ScrollRoot)
            .style(Style::new().node(|node| {
                node.flex_direction = FlexDirection::Row;
                node.overflow = Overflow::clip();
                node.position_type = PositionType::Relative;
            }))
            .children(area.children)
            .into()
    }
}

impl From<ScrollAreaViewport> for View {
    fn from(viewport: ScrollAreaViewport) -> View {
        node()
            .insert(ScrollViewport)
            .insert(ScrollPosition::default())
            .style(Style::new().node(|node| {
                node.overflow = Overflow::scroll_y();
                node.flex_grow = 1.0;
                node.height = Val::Percent(100.0);
                node.flex_direction = FlexDirection::Column;
            }))
            .children(viewport.children)
            .into()
    }
}

impl From<ScrollAreaScrollbar> for View {
    fn from(scrollbar: ScrollAreaScrollbar) -> View {
        node()
            .insert(ScrollBar)
            .style(
                Style::new()
                    .background(color::surface_canvas_hover)
                    .node(|node| {
                        node.width = Val::Px(10.0);
                        node.height = Val::Percent(100.0);
                        node.position_type = PositionType::Relative;
                        node.border_radius = BorderRadius::all(Val::Px(radius::PILL));
                    }),
            )
            .children(scrollbar.children)
            .into()
    }
}

impl From<ScrollAreaThumb> for View {
    fn from(_: ScrollAreaThumb) -> View {
        node()
            .insert(ScrollThumbMark)
            .style(
                Style::new()
                    .background(color::surface_canvas_border)
                    .node(|node| {
                        node.position_type = PositionType::Absolute;
                        node.left = Val::Px(2.0);
                        node.width = Val::Px(6.0);
                        node.border_radius = BorderRadius::all(Val::Px(radius::PILL));
                    }),
            )
            .into()
    }
}

impl From<ScrollAreaCorner> for View {
    fn from(_: ScrollAreaCorner) -> View {
        node().into()
    }
}

/// Sizes and positions each scroll area's thumb from its viewport: the thumb's length is the fraction of
/// the content that's visible, and its offset is how far the viewport has scrolled. Runs after layout so
/// the measured content size is current.
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
