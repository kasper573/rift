//! `Tabs`: one panel visible at a time. Controlled — `value` is a prop and a trigger requests its own
//! value through `on_value_change`; each `TabsContent` mounts only while its value is the active one.

use std::sync::Arc;

use bevy_color::Color;
use bevy_ecs::prelude::*;
use bevy_ui::{AlignItems, JustifyContent, UiRect, Val};
use bevy_view::{View, gate, node};

use crate::controlled::{OnChange, controlled, node_controller, noop, select, when_selected};
use crate::motion::transition::STANDARD_ENTER;
use crate::recipe::{Paint, Style, Styled};
use crate::theme::color;
use crate::tokens::spacing;

#[derive(Default)]
pub struct Tabs {
    value: Option<String>,
    on_value_change: Option<OnChange<Option<String>>>,
    children: Vec<View>,
}

impl Tabs {
    pub fn value(mut self, value: impl Into<Option<String>>) -> Tabs {
        self.value = value.into();
        self
    }

    pub fn on_value_change<F>(mut self, handler: F) -> Tabs
    where
        F: Fn(&mut World, Option<String>) + Send + Sync + 'static,
    {
        self.on_value_change = Some(Arc::new(handler));
        self
    }
}

children_builder!(Tabs);

/// The row of triggers.
#[derive(Default)]
pub struct TabsList {
    children: Vec<View>,
}

children_builder!(TabsList);

/// Selects its `value` when clicked.
#[derive(Default)]
pub struct TabsTrigger {
    value: String,
    children: Vec<View>,
}

impl TabsTrigger {
    pub fn value(mut self, value: impl Into<String>) -> TabsTrigger {
        self.value = value.into();
        self
    }
}

children_builder!(TabsTrigger);

/// The panel shown while its `value` is active.
#[derive(Default)]
pub struct TabsContent {
    value: String,
    children: Vec<View>,
}

impl TabsContent {
    pub fn value(mut self, value: impl Into<String>) -> TabsContent {
        self.value = value.into();
        self
    }
}

children_builder!(TabsContent);

impl From<Tabs> for View {
    fn from(tabs: Tabs) -> View {
        node_controller(
            tabs.value,
            tabs.on_value_change.unwrap_or_else(noop),
            tabs.children,
        )
    }
}

impl From<TabsList> for View {
    fn from(list: TabsList) -> View {
        let style = Style::new()
            .node(|node| {
                node.flex_direction = bevy_ui::FlexDirection::Row;
                node.width = bevy_ui::Val::Percent(100.0);
                node.border = UiRect {
                    left: bevy_ui::Val::Px(0.0),
                    right: bevy_ui::Val::Px(0.0),
                    top: bevy_ui::Val::Px(0.0),
                    bottom: bevy_ui::Val::Px(1.0),
                };
            })
            .border_color(color::surface_canvas_border_decorative);
        node().style(style).children(list.children).into()
    }
}

impl From<TabsTrigger> for View {
    fn from(trigger: TabsTrigger) -> View {
        let value = trigger.value.clone();
        node()
            .attr(move |entity| {
                let id = entity.id();
                let selected = entity.world_scope(|world| is_selected(world, id, &value));
                trigger_style(selected).apply(entity);
            })
            .on_click_with(select(trigger.value))
            .children(trigger.children)
            .into()
    }
}

impl From<TabsContent> for View {
    fn from(content: TabsContent) -> View {
        gate(
            when_selected(content.value),
            node().children(content.children),
        )
    }
}

fn is_selected(world: &World, host: Entity, value: &str) -> bool {
    controlled::<Option<String>>(world, host)
        .and_then(|control| control.value)
        .as_deref()
        == Some(value)
}

/// A trigger: wide padding, surface fill with hover/press shades, and a 2px bottom rule that turns the
/// primary color (and the label the selected color) while it owns the active panel.
fn trigger_style(selected: bool) -> Style {
    let (text, underline): (Paint, Paint) = if selected {
        (color::secondary_on.into(), color::primary_base.into())
    } else {
        (color::surface_canvas_on.into(), Color::NONE.into())
    };
    Style::new()
        .node(|node| {
            node.padding = UiRect::axes(Val::Px(spacing::XXXL), Val::Px(spacing::L));
            node.border = UiRect::bottom(Val::Px(2.0));
            node.align_items = AlignItems::End;
            node.justify_content = JustifyContent::Center;
        })
        .background(color::surface_canvas_base)
        .text_color(text)
        .border_color(underline)
        .transition(STANDARD_ENTER)
        .hover(Style::new().background(color::surface_canvas_hover))
        .active(Style::new().background(color::surface_canvas_active))
}
