use std::collections::HashSet;

use bevy_ecs::bundle::Bundle;
use bevy_ui::{
    AlignItems, BorderRadius, FlexDirection, JustifyContent, Node, Overflow, UiRect, Val,
};
use bevy_ui_widgets::Button;

use crate::collapse::Collapse;
use crate::motion::transition::STANDARD_ENTER;
use crate::state::{SelectGroup, SelectItem, SelectTrigger};
use crate::style::{StatefulPaint, Style};
use crate::theme::color;
use crate::tokens::{radius, spacing};

pub fn accordion(value: HashSet<String>, multiple: bool) -> impl Bundle {
    (
        Node::default(),
        SelectGroup {
            exclusive: !multiple,
            toggleable: true,
            initial: value.into_iter().collect(),
        },
        card_style(),
        crate::surface::elevation(1),
    )
}

pub fn accordion_item() -> impl Bundle {
    (
        Node::default(),
        Style::new()
            .border_color(color::surface_canvas.border)
            .node(|node| {
                node.flex_direction = FlexDirection::Column;
                node.width = Val::Percent(100.0);
                node.border = UiRect::bottom(Val::Px(1.0));
            }),
    )
}

pub fn accordion_header() -> impl Bundle {
    (
        Node::default(),
        Style::new().node(|node| {
            node.flex_direction = FlexDirection::Column;
            node.width = Val::Percent(100.0);
        }),
    )
}

pub fn accordion_trigger(value: impl Into<String>) -> impl Bundle {
    (
        Node::default(),
        Button,
        SelectItem {
            value: value.into(),
        },
        SelectTrigger,
        Style::new()
            .text_color(color::surface_canvas.on)
            .background(
                StatefulPaint::new(color::surface_elevated.base).hover(color::surface_canvas.hover),
            )
            .transition(STANDARD_ENTER)
            .node(|node| {
                node.width = Val::Percent(100.0);
                node.flex_direction = FlexDirection::Row;
                node.justify_content = JustifyContent::SpaceBetween;
                node.align_items = AlignItems::Center;
                node.padding = UiRect::axes(Val::Px(spacing::XL), Val::Px(spacing::L));
            }),
    )
}

pub fn accordion_content(value: impl Into<String>) -> impl Bundle {
    (
        Node {
            overflow: Overflow::clip(),
            width: Val::Percent(100.0),
            ..Node::default()
        },
        SelectItem {
            value: value.into(),
        },
        Collapse::default(),
    )
}

pub fn accordion_body() -> impl Bundle {
    (
        Node::default(),
        Style::new()
            .text_color(color::surface_canvas.on)
            .node(|node| {
                node.padding = UiRect::new(
                    Val::Px(spacing::XL),
                    Val::Px(spacing::XL),
                    Val::Px(0.0),
                    Val::Px(spacing::L),
                );
            }),
    )
}

fn card_style() -> Style {
    Style::new()
        .background(color::surface_elevated.base)
        .node(|node| {
            node.flex_direction = FlexDirection::Column;
            node.width = Val::Percent(100.0);
            node.border_radius = BorderRadius::all(Val::Px(radius::M));
        })
}
