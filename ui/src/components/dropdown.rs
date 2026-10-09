use bevy_color::Color;
use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::prelude::*;
use bevy_math::Rot2;
use bevy_scene::{EntityScene, Scene, bsn, template_value};
use bevy_ui::widget::Text;
use bevy_ui::{
    AlignItems, BorderColor, BorderRadius, Checkable, Checked, FlexDirection, JustifyContent, Node,
    UiRect, UiTransform, Val,
};
use bevy_ui_widgets::{Activate, Button, ValueChange};

use crate::components::popover::popover_content;
use crate::components::text::text;
use crate::motion::transition::STANDARD_ENTER;
use crate::overlay::{Dismissable, Open, OverlayAction};
use crate::state::{Gated, InheritChecked, SelectGroup, SelectItem, SelectTrigger, ancestor_with};
use crate::style::{StatefulPaint, Style};
use crate::theme::theme;
use crate::tokens::{radius, size, spacing};
use crate::{Align, Side, component};

#[derive(Component, Default, Clone)]
#[require(Node)]
pub(crate) struct Dropdown;

#[derive(Component, Default, Clone)]
#[require(Node)]
pub(crate) struct DropdownChoice {
    label: String,
}

#[derive(Component, Default, Clone)]
#[require(Node)]
pub(crate) struct DropdownValue;

pub fn dropdown(value: impl Into<String>) -> impl Scene {
    bsn! {
        Dropdown
        Open(false)
        Dismissable
        SelectGroup { exclusive: true, toggleable: false, initial: {vec![value.into()]} }
        Node { flex_direction: FlexDirection::Column, width: Val::Percent(100.0) }
    }
}

pub fn dropdown_trigger() -> impl Scene {
    let chevron = theme().surface_canvas.on;
    bsn! {
        Button
        component(OverlayAction::Toggle)
        template_value(trigger_style())
        Children [
            ( {text(String::new())} DropdownValue ),
            (
                Node {
                    width: Val::Px(7.0),
                    height: Val::Px(7.0),
                    margin: {UiRect::bottom(Val::Px(spacing::M))},
                    border: {UiRect::new(Val::Px(0.0), Val::Px(2.0), Val::Px(0.0), Val::Px(2.0))},
                }
                UiTransform { rotation: {Rot2::degrees(45.0)} }
                component(BorderColor::all(chevron))
            ),
        ]
    }
}

pub fn dropdown_content() -> impl Scene {
    bsn! {
        {popover_content(Side::Bottom, Align::Start, spacing::M)}
        template_value(content_style())
        component(crate::surface::elevation(1))
    }
}

pub fn dropdown_item(value: impl Into<String>, label: impl Into<String>) -> impl Scene {
    let label = label.into();
    bsn! {
        Button
        Checkable
        SelectItem { value: {value.into()} }
        SelectTrigger
        DropdownChoice { label: {label.clone()} }
        component(OverlayAction::Close)
        template_value(item_style())
        Children [
            {EntityScene(text(label))},
            (
                Node {
                    width: Val::Px(size::STEP_200),
                    height: Val::Px(size::STEP_200),
                    border_radius: {BorderRadius::all(Val::Px(radius::PILL))},
                }
                InheritChecked
                Gated
                template_value(Style::new().background(theme().primary.base))
            ),
        ]
    }
}

pub(crate) fn announce_choice(
    activate: On<Activate>,
    choices: Query<&SelectItem, With<DropdownChoice>>,
    parents: Query<&ChildOf>,
    is_dropdown: Query<(), With<Dropdown>>,
    mut commands: Commands,
) {
    let Ok(choice) = choices.get(activate.entity) else {
        return;
    };
    let Some(dropdown) = ancestor_with::<Dropdown>(activate.entity, &parents, &is_dropdown) else {
        return;
    };
    commands.trigger(ValueChange {
        source: dropdown,
        value: choice.value.clone(),
        is_final: true,
    });
}

pub(crate) fn sync_dropdown_values(
    choices: Query<(Entity, &DropdownChoice), With<Checked>>,
    parents: Query<&ChildOf>,
    is_dropdown: Query<(), With<Dropdown>>,
    mut values: Query<(Entity, &mut Text), With<DropdownValue>>,
) {
    for (entity, mut shown) in &mut values {
        let owner = ancestor_with::<Dropdown>(entity, &parents, &is_dropdown);
        let picked = choices.iter().find(|(choice, _)| {
            owner.is_some() && ancestor_with::<Dropdown>(*choice, &parents, &is_dropdown) == owner
        });
        if let Some((_, choice)) = picked
            && shown.0 != choice.label
        {
            shown.0.clone_from(&choice.label);
        }
    }
}

fn trigger_style() -> Style {
    Style::new()
        .text_color(theme().surface_canvas.on)
        .background(
            StatefulPaint::new(theme().surface_elevated.base)
                .hover(theme().surface_canvas.hover)
                .active(theme().surface_canvas.active),
        )
        .border_color(StatefulPaint::new(theme().surface_canvas.border))
        .transition(STANDARD_ENTER)
        .node(|node| {
            node.width = Val::Percent(100.0);
            node.height = Val::Px(size::STEP_1000);
            node.flex_direction = FlexDirection::Row;
            node.justify_content = JustifyContent::SpaceBetween;
            node.align_items = AlignItems::Center;
            node.column_gap = Val::Px(spacing::L);
            node.padding = UiRect::horizontal(Val::Px(spacing::XL));
            node.border = UiRect::all(Val::Px(1.0));
            node.border_radius = BorderRadius::all(Val::Px(radius::S));
        })
}

fn content_style() -> Style {
    Style::new()
        .background(theme().surface_elevated.base)
        .border_color(StatefulPaint::new(theme().surface_canvas.border))
        .node(|node| {
            node.flex_direction = FlexDirection::Column;
            node.min_width = Val::Percent(100.0);
            node.padding = UiRect::all(Val::Px(spacing::M));
            node.row_gap = Val::Px(spacing::S);
            node.border = UiRect::all(Val::Px(1.0));
            node.border_radius = BorderRadius::all(Val::Px(radius::M));
        })
}

fn item_style() -> Style {
    Style::new()
        .text_color(theme().surface_canvas.on)
        .background(
            StatefulPaint::new(Color::NONE)
                .hover(theme().surface_canvas.hover)
                .active(theme().surface_canvas.active),
        )
        .transition(STANDARD_ENTER)
        .node(|node| {
            node.width = Val::Percent(100.0);
            node.flex_direction = FlexDirection::Row;
            node.justify_content = JustifyContent::SpaceBetween;
            node.align_items = AlignItems::Center;
            node.column_gap = Val::Px(spacing::XL);
            node.padding = UiRect::axes(Val::Px(spacing::L), Val::Px(spacing::M + spacing::S));
            node.border_radius = BorderRadius::all(Val::Px(radius::S));
        })
}
