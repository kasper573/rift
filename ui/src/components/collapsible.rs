use std::sync::Arc;

use bevy_ecs::prelude::*;
use bevy_ui::{AlignItems, FlexDirection, JustifyContent, UiRect, Val};
use bevy_view::{View, node};

use crate::collapse::collapse;
use crate::controlled::{OnChange, controlled, flip, node_controller, noop};
use crate::motion::transition::STANDARD_ENTER;
use crate::recipe::{Style, Styled};
use crate::theme::color;
use crate::tokens::spacing;

#[derive(Default)]
pub struct Collapsible {
    open: bool,
    on_open_change: Option<OnChange<bool>>,
    children: Vec<View>,
}

impl Collapsible {
    pub fn open(mut self, open: bool) -> Collapsible {
        self.open = open;
        self
    }

    pub fn on_open_change<F>(mut self, handler: F) -> Collapsible
    where
        F: Fn(&mut World, bool) + Send + Sync + 'static,
    {
        self.on_open_change = Some(Arc::new(handler));
        self
    }
}

children_builder!(Collapsible);

#[derive(Default)]
pub struct CollapsibleTrigger {
    children: Vec<View>,
}

children_builder!(CollapsibleTrigger);

#[derive(Default)]
pub struct CollapsibleContent {
    children: Vec<View>,
}

children_builder!(CollapsibleContent);

impl From<Collapsible> for View {
    fn from(collapsible: Collapsible) -> View {
        node_controller(
            collapsible.open,
            collapsible.on_open_change.unwrap_or_else(noop),
            collapsible.children,
        )
    }
}

impl From<CollapsibleTrigger> for View {
    fn from(trigger: CollapsibleTrigger) -> View {
        node()
            .style(trigger_style())
            .on_click_with(flip)
            .children(trigger.children)
            .into()
    }
}

impl From<CollapsibleContent> for View {
    fn from(content: CollapsibleContent) -> View {
        collapse(
            |world, entity| controlled::<bool>(world, entity).is_some_and(|control| control.value),
            vec![
                node()
                    .style(content_style())
                    .children(content.children)
                    .into(),
            ],
        )
        .into()
    }
}

fn trigger_style() -> Style {
    Style::new()
        .text_color(color::surface_canvas_on)
        .transition(STANDARD_ENTER)
        .hover(Style::new().background(color::surface_canvas_hover))
        .node(|node| {
            node.width = Val::Percent(100.0);
            node.flex_direction = FlexDirection::Row;
            node.justify_content = JustifyContent::SpaceBetween;
            node.align_items = AlignItems::Center;
            node.padding = UiRect::axes(Val::Px(spacing::M), Val::Px(spacing::L));
        })
}

fn content_style() -> Style {
    Style::new()
        .text_color(color::surface_canvas_on_soft)
        .node(|node| {
            node.padding = UiRect::new(
                Val::Px(spacing::M),
                Val::Px(spacing::M),
                Val::Px(spacing::S),
                Val::Px(spacing::L),
            );
        })
}
