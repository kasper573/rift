use bevy_ecs::bundle::Bundle;
use bevy_ui::{AlignItems, FlexDirection, JustifyContent, Node, Overflow, UiRect, Val};
use bevy_ui_widgets::Button;

use crate::collapse::Collapse;
use crate::motion::transition::STANDARD_ENTER;
use crate::state::{SelectGroup, SelectItem, SelectTrigger};
use crate::style::Style;
use crate::theme::color;
use crate::tokens::spacing;

const OPEN: &str = "open";

pub fn collapsible(open: bool) -> impl Bundle {
    (
        Node::default(),
        SelectGroup {
            exclusive: false,
            toggleable: true,
            initial: open.then(|| OPEN.to_owned()).into_iter().collect(),
        },
        Style::new().node(|node| {
            node.flex_direction = FlexDirection::Column;
            node.width = Val::Percent(100.0);
            node.row_gap = Val::Px(spacing::L);
        }),
    )
}

pub fn collapsible_trigger() -> impl Bundle {
    (
        Node::default(),
        Button,
        SelectItem {
            value: OPEN.to_owned(),
        },
        SelectTrigger,
        Style::new()
            .text_color(color::surface_canvas.on)
            .transition(STANDARD_ENTER)
            .hover(Style::new().background(color::surface_canvas.hover))
            .node(|node| {
                node.width = Val::Percent(100.0);
                node.flex_direction = FlexDirection::Row;
                node.justify_content = JustifyContent::SpaceBetween;
                node.align_items = AlignItems::Center;
                node.padding = UiRect::axes(Val::Px(spacing::M), Val::Px(spacing::L));
            }),
    )
}

pub fn collapsible_content() -> impl Bundle {
    (
        Node {
            overflow: Overflow::clip(),
            width: Val::Percent(100.0),
            ..Node::default()
        },
        SelectItem {
            value: OPEN.to_owned(),
        },
        Collapse::default(),
    )
}

pub fn collapsible_body() -> impl Bundle {
    (
        Node::default(),
        Style::new()
            .text_color(color::surface_canvas.on)
            .node(|node| {
                node.padding = UiRect::new(
                    Val::Px(spacing::M),
                    Val::Px(spacing::M),
                    Val::Px(spacing::S),
                    Val::Px(spacing::L),
                );
            }),
    )
}
