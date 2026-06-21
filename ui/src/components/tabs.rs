use bevy_color::Color;
use bevy_ecs::bundle::Bundle;
use bevy_ui::{AlignItems, FlexDirection, JustifyContent, Node, UiRect, Val};
use bevy_ui_widgets::Button;

use crate::motion::transition::STANDARD_ENTER;
use crate::state::{Gated, SelectGroup, SelectItem, SelectTrigger};
use crate::style::Style;
use crate::theme::color;
use crate::tokens::spacing;

pub fn tabs(value: Option<String>) -> impl Bundle {
    (
        Node::default(),
        SelectGroup {
            exclusive: true,
            toggleable: false,
            initial: value.into_iter().collect(),
        },
        Style::new().node(|node| {
            node.flex_direction = FlexDirection::Column;
            node.width = Val::Percent(100.0);
        }),
    )
}

pub fn tabs_list() -> impl Bundle {
    (
        Node::default(),
        Style::new()
            .node(|node| {
                node.flex_direction = FlexDirection::Row;
                node.width = Val::Percent(100.0);
                node.border = UiRect::bottom(Val::Px(1.0));
            })
            .border_color(color::surface_canvas_border_decorative),
    )
}

pub fn tabs_trigger(value: impl Into<String>) -> impl Bundle {
    (
        Node::default(),
        Button,
        SelectItem {
            value: value.into(),
        },
        SelectTrigger,
        trigger_style(),
    )
}

pub fn tabs_content(value: impl Into<String>) -> impl Bundle {
    (
        Node::default(),
        SelectItem {
            value: value.into(),
        },
        Gated,
    )
}

fn trigger_style() -> Style {
    Style::new()
        .node(|node| {
            node.padding = UiRect::axes(Val::Px(spacing::XXXL), Val::Px(spacing::L));
            node.border = UiRect::bottom(Val::Px(2.0));
            node.align_items = AlignItems::End;
            node.justify_content = JustifyContent::Center;
        })
        .background(color::surface_canvas_base)
        .text_color(color::surface_canvas_on)
        .border_color(Color::NONE)
        .transition(STANDARD_ENTER)
        .hover(Style::new().background(color::surface_canvas_hover))
        .active(Style::new().background(color::surface_canvas_active))
        .checked(
            Style::new()
                .text_color(color::secondary_on)
                .border_color(color::primary_base),
        )
}
