use bevy_color::Color;
use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::prelude::*;
use bevy_ui::{
    AlignItems, BorderRadius, BoxShadow, ComputedNode, Display, Node, PositionType, ShadowStyle,
    UiGlobalTransform, Val,
};
use bevy_ui_widgets::ValueChange;
use bevy_window::{PrimaryWindow, Window};

use bevy_math::Vec2;
use bevy_picking::prelude::{Drag, Pointer};

use crate::state::ancestor_with;
use crate::style::Style;
use crate::theme::color;
use crate::tokens::{radius, size};

#[derive(Component)]
pub struct SliderState {
    pub value: f32,
    pub min: f32,
    pub max: f32,
}

#[derive(Component)]
pub struct SliderRange;

#[derive(Component)]
pub struct SliderThumb;

pub fn slider(value: f32, min: f32, max: f32) -> impl Bundle {
    let max = if max > 0.0 { max } else { 100.0 };
    (
        Node::default(),
        SliderState {
            value: value.clamp(min, max),
            min,
            max,
        },
        Style::new().node(|node| {
            node.width = Val::Percent(100.0);
            node.height = Val::Px(size::STEP_600);
            node.display = Display::Flex;
            node.align_items = AlignItems::Center;
        }),
    )
}

pub fn slider_track() -> impl Bundle {
    (
        Node::default(),
        Style::new().background(color::scrim_dark).node(|node| {
            node.flex_grow = 1.0;
            node.height = Val::Px(size::STEP_100);
            node.border_radius = BorderRadius::all(Val::Px(radius::PILL));
            node.position_type = PositionType::Relative;
        }),
    )
}

pub fn slider_range() -> impl Bundle {
    (
        Node::default(),
        SliderRange,
        Style::new().background(color::primary.base).node(|node| {
            node.position_type = PositionType::Absolute;
            node.left = Val::Px(0.0);
            node.height = Val::Percent(100.0);
            node.border_radius = BorderRadius::all(Val::Px(radius::PILL));
        }),
    )
}

pub fn slider_thumb() -> impl Bundle {
    (
        Node::default(),
        SliderThumb,
        BoxShadow(vec![ShadowStyle {
            color: Color::srgba(0.0, 0.0, 0.0, 0.28),
            x_offset: Val::Px(0.0),
            y_offset: Val::Px(2.0),
            spread_radius: Val::Px(0.0),
            blur_radius: Val::Px(6.0),
        }]),
        Style::new()
            .background(color::surface_canvas.base)
            .node(|node| {
                node.width = Val::Px(size::STEP_600);
                node.height = Val::Px(size::STEP_600);
                node.border_radius = BorderRadius::all(Val::Px(radius::L));
                node.position_type = PositionType::Absolute;
                node.top = Val::Px(-10.0);
            })
            .translate(Vec2::new(-(size::STEP_600 / 2.0), 0.0)),
    )
}

fn fraction(state: &SliderState) -> f32 {
    ((state.value - state.min) / (state.max - state.min)).clamp(0.0, 1.0)
}

#[allow(clippy::type_complexity)]
pub(crate) fn sync_slider(
    states: Query<&SliderState>,
    parents: Query<&ChildOf>,
    has_state: Query<(), With<SliderState>>,
    mut ranges: Query<(Entity, &mut Node), (With<SliderRange>, Without<SliderThumb>)>,
    mut thumbs: Query<(Entity, &mut Node), (With<SliderThumb>, Without<SliderRange>)>,
) {
    let fraction_of = |entity: Entity| {
        ancestor_with::<SliderState>(entity, &parents, &has_state)
            .and_then(|root| states.get(root).ok())
            .map_or(0.0, fraction)
    };
    for (entity, mut node) in &mut ranges {
        node.width = Val::Percent(fraction_of(entity) * 100.0);
    }
    for (entity, mut node) in &mut thumbs {
        node.left = Val::Percent(fraction_of(entity) * 100.0);
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn on_thumb_drag(
    drag: On<Pointer<Drag>>,
    thumbs: Query<(), With<SliderThumb>>,
    parents: Query<&ChildOf>,
    has_state: Query<(), With<SliderState>>,
    mut states: Query<&mut SliderState>,
    measured: Query<(&ComputedNode, &UiGlobalTransform)>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut commands: Commands,
) {
    let thumb = drag.entity;
    if !thumbs.contains(thumb) {
        return;
    }
    let (Some(root), Ok(track)) = (
        ancestor_with::<SliderState>(thumb, &parents, &has_state),
        parents.get(thumb).map(ChildOf::parent),
    ) else {
        return;
    };
    let Ok((track_node, track_transform)) = measured.get(track) else {
        return;
    };
    // Physical pixels (cursor and track) so display scale doesn't affect the mapping.
    let width = track_node.size.x;
    let left = track_transform.translation.x - width / 2.0;
    let Some(cursor) = windows.iter().next().and_then(Window::cursor_position) else {
        return;
    };
    if width <= 0.0 {
        return;
    }
    let fraction = ((cursor.x - left) / width).clamp(0.0, 1.0);
    if let Ok(mut state) = states.get_mut(root) {
        let next = state.min + fraction * (state.max - state.min);
        state.value = next;
        commands.trigger(ValueChange {
            source: root,
            value: next,
            is_final: false,
        });
    }
}
