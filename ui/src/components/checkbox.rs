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

// A checkmark drawn as a right+bottom border corner rotated 45° (the design fonts have no ✓ glyph).
// Painted in the face color so it reads on the filled box.
pub fn checkbox_indicator() -> impl Bundle {
    (
        Node {
            width: Val::Px(6.0),
            height: Val::Px(11.0),
            border: UiRect {
                right: Val::Px(2.0),
                bottom: Val::Px(2.0),
                ..UiRect::ZERO
            },
            ..Node::default()
        },
        InheritChecked,
        Gated,
        Style::new()
            .border_color(color::primary.on)
            .rotate(std::f32::consts::FRAC_PI_4)
            .translate(bevy_math::Vec2::new(0.0, -1.0)),
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
        .background(color::surface_elevated.base)
        .border_color(color::surface_canvas.border)
        .hover(Style::new().background(color::surface_canvas.hover))
        .active(Style::new().background(color::surface_canvas.active))
        .transition(STANDARD_ENTER)
        .checked(
            Style::new()
                .node(|node| node.border = UiRect::all(Val::Px(0.0)))
                .background(color::primary.base)
                .border_color(color::primary.base)
                .hover(Style::new().background(color::primary.hover))
                .active(Style::new().background(color::primary.active)),
        )
}
