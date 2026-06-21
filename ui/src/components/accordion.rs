use std::collections::HashSet;

use bevy_color::Color;
use bevy_ecs::bundle::Bundle;
use bevy_ui::{
    AlignItems, BorderRadius, BoxShadow, FlexDirection, JustifyContent, Node, Overflow,
    ShadowStyle, UiRect, Val,
};
use bevy_ui_widgets::Button;

use crate::collapse::Collapse;
use crate::motion::transition::STANDARD_ENTER;
use crate::state::{SelectGroup, SelectItem, SelectTrigger};
use crate::style::Style;
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
        card_shadow(),
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
            // Resting background (matches the card) so the hover paint has a value to ease back to;
            // without it the Motion is never re-aimed and the hover sticks after mouseout.
            .background(color::surface_elevated.base)
            .transition(STANDARD_ENTER)
            .hover(Style::new().background(color::surface_canvas.hover))
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

// Surface card uses an elevation shadow, not a 1px border: borders on rounded boxes show white at corners.
fn card_style() -> Style {
    Style::new()
        .background(color::surface_elevated.base)
        .node(|node| {
            node.flex_direction = FlexDirection::Column;
            node.width = Val::Percent(100.0);
            node.border_radius = BorderRadius::all(Val::Px(radius::M));
        })
}

fn card_shadow() -> BoxShadow {
    BoxShadow(vec![
        ShadowStyle {
            color: Color::srgba(0.0, 0.0, 0.0, 0.08),
            x_offset: Val::Px(0.0),
            y_offset: Val::Px(1.0),
            spread_radius: Val::Px(0.0),
            blur_radius: Val::Px(2.0),
        },
        ShadowStyle {
            color: Color::srgba(0.0, 0.0, 0.0, 0.08),
            x_offset: Val::Px(0.0),
            y_offset: Val::Px(4.0),
            spread_radius: Val::Px(0.0),
            blur_radius: Val::Px(12.0),
        },
    ])
}
