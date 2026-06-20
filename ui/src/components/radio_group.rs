use std::sync::Arc;

use bevy_ecs::prelude::*;
use bevy_ui::{AlignItems, BorderRadius, FlexDirection, JustifyContent, UiRect, Val};
use bevy_view::{InstanceId, View, context, gate, node, provide};

use crate::controlled::{ItemValue, OnChange, controlled, node_controller, noop, select};
use crate::motion::transition::STANDARD_ENTER;
use crate::recipe::{Style, Styled};
use crate::theme::color;
use crate::tokens::{radius, size, spacing};

#[derive(Default)]
pub struct RadioGroup {
    value: Option<String>,
    on_value_change: Option<OnChange<Option<String>>>,
    children: Vec<View>,
}

impl RadioGroup {
    pub fn value(mut self, value: impl Into<Option<String>>) -> RadioGroup {
        self.value = value.into();
        self
    }

    pub fn on_value_change<F>(mut self, handler: F) -> RadioGroup
    where
        F: Fn(&mut World, Option<String>) + Send + Sync + 'static,
    {
        self.on_value_change = Some(Arc::new(handler));
        self
    }
}

children_builder!(RadioGroup);

#[derive(Default)]
pub struct RadioGroupItem {
    value: String,
    label: Option<View>,
    children: Vec<View>,
}

impl RadioGroupItem {
    pub fn value(mut self, value: impl Into<String>) -> RadioGroupItem {
        self.value = value.into();
        self
    }

    pub fn label(mut self, label: impl Into<View>) -> RadioGroupItem {
        self.label = Some(label.into());
        self
    }
}

children_builder!(RadioGroupItem);

#[derive(Default)]
pub struct RadioGroupIndicator {
    children: Vec<View>,
}

children_builder!(RadioGroupIndicator);

impl From<RadioGroup> for View {
    fn from(group: RadioGroup) -> View {
        node_controller(
            group.value,
            group.on_value_change.unwrap_or_else(noop),
            group.children,
        )
    }
}

impl From<RadioGroupItem> for View {
    fn from(item: RadioGroupItem) -> View {
        let value = item.value;
        let circle = node()
            .attr(|entity| {
                let id = entity.id();
                let sel = entity.world_scope(|world| is_selected(world, id));
                radio_style(sel).apply(entity);
            })
            .children(item.children);
        let mut row = node()
            .style(item_row_style())
            .bind(provide(ItemValue(value.clone())))
            .on_click_with(select(value))
            .child(circle);
        if let Some(label) = item.label {
            row = row.child(label);
        }
        row.into()
    }
}

fn item_row_style() -> Style {
    Style::new().node(|node| {
        node.flex_direction = FlexDirection::Row;
        node.align_items = AlignItems::Center;
        node.column_gap = Val::Px(spacing::M);
    })
}

impl From<RadioGroupIndicator> for View {
    fn from(indicator: RadioGroupIndicator) -> View {
        gate(
            selected,
            node().style(indicator_style()).children(indicator.children),
        )
    }
}

fn selected(world: &World, _: InstanceId, host: Entity) -> bool {
    is_selected(world, host)
}

fn is_selected(world: &World, host: Entity) -> bool {
    let Some(mine) = context::<ItemValue>(world, host) else {
        return false;
    };
    controlled::<Option<String>>(world, host)
        .and_then(|control| control.value)
        .as_deref()
        == Some(mine.0.as_str())
}

fn radio_style(selected: bool) -> Style {
    let (base, hover, active) = if selected {
        (
            color::primary_base,
            color::primary_hover,
            color::primary_active,
        )
    } else {
        (
            color::surface_elevated_base,
            color::surface_canvas_hover,
            color::surface_canvas_active,
        )
    };
    // When selected, drop the border — bordered rounded boxes leak surface at corners (bevy quirk).
    let (border_width, border) = if selected {
        (0.0, color::primary_base)
    } else {
        (2.0, color::surface_canvas_border)
    };
    Style::new()
        .node(move |node| {
            node.width = Val::Px(size::STEP_600);
            node.height = Val::Px(size::STEP_600);
            node.min_width = Val::Px(size::STEP_600);
            node.min_height = Val::Px(size::STEP_600);
            node.border = UiRect::all(Val::Px(border_width));
            node.border_radius = BorderRadius::all(Val::Px(radius::PILL));
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
