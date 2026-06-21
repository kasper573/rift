use bevy_ecs::bundle::Bundle;
use bevy_ui::{AlignItems, BorderRadius, Checkable, JustifyContent, Node, UiRect, Val};
use bevy_ui_widgets::{Checkbox, checkbox_self_update, observe};

use crate::motion::transition::STANDARD_ENTER;
use crate::state::{Gated, InheritChecked, StartChecked};
use crate::style::Style;
use crate::theme::color;
use crate::tokens::{radius, size};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Check {
    #[default]
    Off,
    On,
    Indeterminate,
}

pub fn checkbox(checked: Check) -> impl Bundle {
    (
        Node::default(),
        Checkbox,
        Checkable,
        observe(checkbox_self_update),
        StartChecked(checked != Check::Off),
        box_style(),
    )
}

pub fn checkbox_indicator() -> impl Bundle {
    (
        Node::default(),
        InheritChecked,
        Gated,
        Style::new().text_color(color::primary_on).node(|node| {
            node.width = Val::Percent(100.0);
            node.height = Val::Percent(100.0);
            node.align_items = AlignItems::Center;
            node.justify_content = JustifyContent::Center;
        }),
    )
}

// Filled box drops its border: a bordered rounded box leaks surface at the corners (bevy quirk).
fn box_style() -> Style {
    Style::new()
        .node(|node| {
            node.width = Val::Px(size::STEP_600);
            node.height = Val::Px(size::STEP_600);
            node.min_width = Val::Px(size::STEP_600);
            node.min_height = Val::Px(size::STEP_600);
            node.border = UiRect::all(Val::Px(2.0));
            node.border_radius = BorderRadius::all(Val::Px(radius::S));
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
