use bevy_math::Vec2;
use bevy_scene::{Scene, bsn, on, template_value};
use bevy_ui::{BorderRadius, Checkable, PositionType, UiRect, Val};
use bevy_ui_widgets::{Checkbox, checkbox_self_update};

use crate::motion::transition::STANDARD_ENTER;
use crate::motion::{Easing, Timing};
use crate::state::{InheritChecked, StartChecked};
use crate::style::Style;
use crate::theme::theme;
use crate::tokens::{radius, size};

pub fn switch(checked: bool) -> impl Scene {
    bsn! {
        Checkbox
        Checkable
        on(checkbox_self_update)
        StartChecked({checked})
        template_value(track_style())
    }
}

pub fn switch_thumb() -> impl Scene {
    bsn! {
        InheritChecked
        template_value(thumb_style())
    }
}

fn track_style() -> Style {
    Style::new()
        .node(|node| {
            node.width = Val::Px(52.0);
            node.height = Val::Px(32.0);
            node.border = UiRect::all(Val::Px(2.0));
            node.border_radius = BorderRadius::all(Val::Px(radius::PILL));
            node.position_type = PositionType::Relative;
        })
        .background(theme().surface_elevated.base)
        .border_color(theme().surface_canvas.border)
        .transition(STANDARD_ENTER)
        .checked(
            Style::new()
                .node(|node| node.border = UiRect::all(Val::Px(0.0)))
                .background(theme().primary.base),
        )
}

fn thumb_style() -> Style {
    Style::new()
        .node(|node| {
            node.width = Val::Px(size::STEP_600);
            node.height = Val::Px(size::STEP_600);
            node.position_type = PositionType::Absolute;
            node.top = Val::Px(2.0);
            node.left = Val::Px(2.0);
            node.border_radius = BorderRadius::all(Val::Px(radius::PILL));
        })
        .background(theme().surface_canvas.border)
        .translate(Vec2::ZERO)
        .transition(Timing::new(150, Easing::Standard))
        .checked(
            Style::new()
                .node(|node| {
                    node.top = Val::Px(4.0);
                    node.left = Val::Px(4.0);
                })
                .background(theme().surface_floating.base)
                .translate(Vec2::new(20.0, 0.0)),
        )
}
