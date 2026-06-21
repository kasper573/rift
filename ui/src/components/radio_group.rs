use bevy_ecs::bundle::Bundle;
use bevy_ui::{
    AlignItems, BorderRadius, Checkable, FlexDirection, JustifyContent, Node, UiRect, Val,
};
use bevy_ui_widgets::Button;

use crate::motion::transition::STANDARD_ENTER;
use crate::state::{Gated, InheritChecked, SelectGroup, SelectItem, SelectTrigger};
use crate::style::Style;
use crate::theme::color;
use crate::tokens::{radius, size, spacing};

pub fn radio_group(value: Option<String>) -> impl Bundle {
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
            node.row_gap = Val::Px(spacing::L);
        }),
    )
}

pub fn radio_item(value: impl Into<String>) -> impl Bundle {
    (
        Node::default(),
        Button,
        Checkable,
        SelectItem {
            value: value.into(),
        },
        SelectTrigger,
        Style::new().node(|node| {
            node.flex_direction = FlexDirection::Row;
            node.align_items = AlignItems::Center;
            node.column_gap = Val::Px(spacing::M);
        }),
    )
}

pub fn radio_circle() -> impl Bundle {
    (Node::default(), InheritChecked, circle_style())
}

// Selected dot, painted in the face color (the design fonts have no ● glyph).
pub fn radio_indicator() -> impl Bundle {
    (
        Node {
            width: Val::Px(10.0),
            height: Val::Px(10.0),
            border_radius: BorderRadius::all(Val::Px(radius::PILL)),
            ..Node::default()
        },
        InheritChecked,
        Gated,
        Style::new().background(color::primary_on),
    )
}

// Filled circle drops its border: a bordered rounded box leaks surface at the corners (bevy quirk).
fn circle_style() -> Style {
    Style::new()
        .node(|node| {
            node.width = Val::Px(size::STEP_600);
            node.height = Val::Px(size::STEP_600);
            node.min_width = Val::Px(size::STEP_600);
            node.min_height = Val::Px(size::STEP_600);
            node.border = UiRect::all(Val::Px(2.0));
            node.border_radius = BorderRadius::all(Val::Px(radius::PILL));
            node.align_items = AlignItems::Center;
            node.justify_content = JustifyContent::Center;
        })
        .background(color::surface_elevated_base)
        .border_color(color::surface_canvas_border)
        .hover(Style::new().background(color::surface_canvas_hover))
        .active(Style::new().background(color::surface_canvas_active))
        .transition(STANDARD_ENTER)
        .checked(
            Style::new()
                .node(|node| node.border = UiRect::all(Val::Px(0.0)))
                .background(color::primary_base)
                .border_color(color::primary_base)
                .hover(Style::new().background(color::primary_hover))
                .active(Style::new().background(color::primary_active)),
        )
}
