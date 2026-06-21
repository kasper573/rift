use bevy_color::Color;
use bevy_ecs::bundle::Bundle;
use bevy_ui::{AlignItems, FlexDirection, JustifyContent, Node, UiRect, Val};
use bevy_ui_widgets::Button;

use crate::motion::transition::STANDARD_ENTER;
use crate::state::{Gated, SelectGroup, SelectItem, SelectTrigger};
use crate::style::{StatefulPaint, Style};
use crate::theme::theme;
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
            .border_color(theme().surface_canvas.border),
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
        .background(
            StatefulPaint::new(theme().surface_canvas.base)
                .hover(theme().surface_canvas.hover)
                .active(theme().surface_canvas.active),
        )
        .text_color(StatefulPaint::new(theme().surface_canvas.on).checked(theme().secondary.on))
        .border_color(StatefulPaint::new(Color::NONE).checked(theme().primary.base))
        .transition(STANDARD_ENTER)
}
