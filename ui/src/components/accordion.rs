use std::collections::HashSet;
use std::sync::Arc;

use bevy_color::Color;
use bevy_ecs::prelude::*;
use bevy_ui::{
    AlignItems, BorderRadius, BoxShadow, FlexDirection, JustifyContent, ShadowStyle, UiRect, Val,
};
use bevy_view::{View, context, node, provide};

use crate::collapse::collapse;
use crate::controlled::{
    ItemValue, MultiControlled, OnChange, multi_controller, noop, toggle_scope,
};
use crate::motion::transition::STANDARD_ENTER;
use crate::recipe::{Style, Styled};
use crate::theme::color;
use crate::tokens::{radius, spacing};

#[derive(Default)]
pub struct Accordion {
    value: HashSet<String>,
    multiple: bool,
    on_value_change: Option<OnChange<HashSet<String>>>,
    children: Vec<View>,
}

impl Accordion {
    pub fn value(mut self, value: HashSet<String>) -> Accordion {
        self.value = value;
        self
    }
    pub fn multiple(mut self, multiple: bool) -> Accordion {
        self.multiple = multiple;
        self
    }

    pub fn on_value_change<F>(mut self, handler: F) -> Accordion
    where
        F: Fn(&mut World, HashSet<String>) + Send + Sync + 'static,
    {
        self.on_value_change = Some(Arc::new(handler));
        self
    }
}

children_builder!(Accordion);

#[derive(Default)]
pub struct AccordionItem {
    value: String,
    children: Vec<View>,
}

impl AccordionItem {
    pub fn value(mut self, value: impl Into<String>) -> AccordionItem {
        self.value = value.into();
        self
    }
}

children_builder!(AccordionItem);

#[derive(Default)]
pub struct AccordionHeader {
    children: Vec<View>,
}

children_builder!(AccordionHeader);

#[derive(Default)]
pub struct AccordionTrigger {
    children: Vec<View>,
}

children_builder!(AccordionTrigger);

#[derive(Default)]
pub struct AccordionContent {
    children: Vec<View>,
}

children_builder!(AccordionContent);

impl From<Accordion> for View {
    fn from(accordion: Accordion) -> View {
        let card = node()
            .style(card_style())
            .insert(card_shadow())
            .children(accordion.children);
        multi_controller(
            accordion.value,
            accordion.multiple,
            accordion.on_value_change.unwrap_or_else(noop),
            vec![card.into()],
        )
    }
}

impl From<AccordionItem> for View {
    fn from(item: AccordionItem) -> View {
        node()
            .style(item_style())
            .bind(provide(ItemValue(item.value)))
            .children(item.children)
            .into()
    }
}

impl From<AccordionHeader> for View {
    fn from(header: AccordionHeader) -> View {
        node().style(column()).children(header.children).into()
    }
}

impl From<AccordionTrigger> for View {
    fn from(trigger: AccordionTrigger) -> View {
        node()
            .style(trigger_style())
            .on_click_with(toggle_scope)
            .children(trigger.children)
            .into()
    }
}

impl From<AccordionContent> for View {
    fn from(content: AccordionContent) -> View {
        collapse(
            is_member,
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

fn is_member(world: &World, entity: Entity) -> bool {
    let Some(item) = context::<ItemValue>(world, entity) else {
        return false;
    };
    context::<MultiControlled>(world, entity).is_some_and(|group| group.values.contains(&item.0))
}

fn column() -> Style {
    Style::new().node(|node| {
        node.flex_direction = FlexDirection::Column;
        node.width = Val::Percent(100.0);
    })
}

fn item_style() -> Style {
    Style::new()
        .border_color(color::surface_canvas_border_decorative)
        .node(|node| {
            node.flex_direction = FlexDirection::Column;
            node.width = Val::Percent(100.0);
            node.border = UiRect::bottom(Val::Px(1.0));
        })
}

/// Surface card. Uses elevation shadow instead of 1px border — borders on rounded boxes show white at corners.
fn card_style() -> Style {
    Style::new()
        .background(color::surface_elevated_base)
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
            node.padding = UiRect::axes(Val::Px(spacing::XL), Val::Px(spacing::L));
        })
}

fn content_style() -> Style {
    Style::new()
        .text_color(color::surface_canvas_on_soft)
        .node(|node| {
            node.padding = UiRect::new(
                Val::Px(spacing::XL),
                Val::Px(spacing::XL),
                Val::Px(0.0),
                Val::Px(spacing::L),
            );
        })
}
