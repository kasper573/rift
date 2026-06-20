use std::sync::Arc;

use bevy_ecs::prelude::*;
use bevy_ui::{AlignItems, BorderRadius, JustifyContent, UiRect, Val};
use bevy_view::{InstanceId, View, button, gate, node};

use crate::controlled::{OnChange, controlled, controller, noop};
use crate::motion::transition::STANDARD_ENTER;
use crate::recipe::{Style, Styled};
use crate::theme::color;
use crate::tokens::{radius, size};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Check {
    #[default]
    Off,
    On,
    Indeterminate,
}

#[derive(Default)]
pub struct Checkbox {
    checked: Check,
    on_checked_change: Option<OnChange<Check>>,
    children: Vec<View>,
}

impl Checkbox {
    pub fn checked(mut self, checked: Check) -> Checkbox {
        self.checked = checked;
        self
    }

    pub fn on_checked_change<F>(mut self, handler: F) -> Checkbox
    where
        F: Fn(&mut World, Check) + Send + Sync + 'static,
    {
        self.on_checked_change = Some(Arc::new(handler));
        self
    }
}

children_builder!(Checkbox);

#[derive(Default)]
pub struct CheckboxIndicator {
    children: Vec<View>,
}

children_builder!(CheckboxIndicator);

impl From<Checkbox> for View {
    fn from(checkbox: Checkbox) -> View {
        controller(
            button()
                .style(box_style(checkbox.checked != Check::Off))
                .on_click_with(toggle),
            checkbox.checked,
            checkbox.on_checked_change.unwrap_or_else(noop),
            checkbox.children,
        )
    }
}

impl From<CheckboxIndicator> for View {
    fn from(indicator: CheckboxIndicator) -> View {
        gate(
            checked,
            node().style(indicator_style()).children(indicator.children),
        )
    }
}

fn toggle(world: &mut World, entity: Entity) {
    if let Some(control) = controlled::<Check>(world, entity) {
        let next = if control.value == Check::On {
            Check::Off
        } else {
            Check::On
        };
        control.request(world, next);
    }
}

fn checked(world: &World, _: InstanceId, host: Entity) -> bool {
    controlled::<Check>(world, host).is_some_and(|control| control.value != Check::Off)
}

/// Checkbox box. When filled, drop the border — bordered rounded boxes leak surface at corners (bevy quirk).
fn box_style(on: bool) -> Style {
    let (base, hover, active, border) = if on {
        (
            color::primary_base,
            color::primary_hover,
            color::primary_active,
            color::primary_base,
        )
    } else {
        (
            color::surface_elevated_base,
            color::surface_canvas_hover,
            color::surface_canvas_active,
            color::surface_canvas_border,
        )
    };
    let border_width = if on { 0.0 } else { 2.0 };
    Style::new()
        .node(move |node| {
            node.width = Val::Px(size::STEP_600);
            node.height = Val::Px(size::STEP_600);
            node.min_width = Val::Px(size::STEP_600);
            node.min_height = Val::Px(size::STEP_600);
            node.border = UiRect::all(Val::Px(border_width));
            node.border_radius = BorderRadius::all(Val::Px(radius::S));
            node.align_items = AlignItems::Center;
            node.justify_content = JustifyContent::Center;
        })
        .background(base)
        .border_color(border)
        .hover(Style::new().background(hover))
        .active(Style::new().background(active))
        .transition(STANDARD_ENTER)
}

fn indicator_style() -> Style {
    Style::new().text_color(color::primary_on).node(|node| {
        node.width = Val::Percent(100.0);
        node.height = Val::Percent(100.0);
        node.align_items = AlignItems::Center;
        node.justify_content = JustifyContent::Center;
    })
}
