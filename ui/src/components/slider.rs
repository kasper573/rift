//! `Slider`: a draggable value along a track. Controlled — `value` is a prop and dragging the thumb
//! requests a new value (clamped to `min..max`) through `on_value_change`. The root shares its value and
//! bounds with the thumb through context. The track is 4px high, the thumb is 24px, and the range fills
//! from left based on the value.

use std::sync::Arc;

use bevy_color::Color;
use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::prelude::*;
use bevy_math::Vec2;
use bevy_ui::{
    AlignItems, BorderRadius, BoxShadow, ComputedNode, ShadowStyle, UiGlobalTransform, Val,
};
use bevy_view::{View, context, node, provide};
use bevy_window::{PrimaryWindow, Window};

use crate::controlled::{OnChange, noop};
use crate::recipe::{Style, Styled};
use crate::theme::color;
use crate::tokens::{radius, size};

/// The value and bounds a [`Slider`] shares with its [`SliderThumb`].
#[derive(Clone)]
struct Range {
    value: f32,
    min: f32,
    max: f32,
    on_change: OnChange<f32>,
}

#[derive(Default)]
pub struct Slider {
    value: f32,
    min: f32,
    max: f32,
    on_value_change: Option<OnChange<f32>>,
    children: Vec<View>,
}

impl Slider {
    pub fn value(mut self, value: f32) -> Slider {
        self.value = value;
        self
    }
    pub fn min(mut self, min: f32) -> Slider {
        self.min = min;
        self
    }
    pub fn max(mut self, max: f32) -> Slider {
        self.max = max;
        self
    }

    pub fn on_value_change<F>(mut self, handler: F) -> Slider
    where
        F: Fn(&mut World, f32) + Send + Sync + 'static,
    {
        self.on_value_change = Some(Arc::new(handler));
        self
    }
}

children_builder!(Slider);

/// The rail the thumb travels along.
#[derive(Default)]
pub struct SliderTrack {
    children: Vec<View>,
}

children_builder!(SliderTrack);

/// The filled portion from the start to the thumb (styled by the app).
#[derive(Default)]
pub struct SliderRange;

/// The draggable handle.
#[derive(Default)]
pub struct SliderThumb;

impl From<Slider> for View {
    fn from(slider: Slider) -> View {
        let max = if slider.max > 0.0 { slider.max } else { 100.0 };
        let range = Range {
            value: slider.value.clamp(slider.min, max),
            min: slider.min,
            max,
            on_change: slider.on_value_change.unwrap_or_else(noop),
        };
        node()
            .style(root_style())
            .bind(provide(range))
            .children(slider.children)
            .into()
    }
}

impl From<SliderTrack> for View {
    fn from(track: SliderTrack) -> View {
        let style = track_style();
        node().style(style).children(track.children).into()
    }
}

impl From<SliderRange> for View {
    fn from(_: SliderRange) -> View {
        node()
            .style(range_style())
            .attr(|entity| {
                let id = entity.id();
                let fraction = entity.world_scope(|world| fraction(world, id));
                if let Some(mut node) = entity.get_mut::<bevy_ui::Node>() {
                    node.width = Val::Percent(fraction * 100.0);
                }
            })
            .into()
    }
}

impl From<SliderThumb> for View {
    fn from(_: SliderThumb) -> View {
        node()
            .style(thumb_style())
            .insert(BoxShadow(vec![ShadowStyle {
                color: Color::srgba(0.0, 0.0, 0.0, 0.28),
                x_offset: Val::Px(0.0),
                y_offset: Val::Px(2.0),
                spread_radius: Val::Px(0.0),
                blur_radius: Val::Px(6.0),
            }]))
            .attr(|entity| {
                let id = entity.id();
                let fraction = entity.world_scope(|world| fraction(world, id));
                if let Some(mut node) = entity.get_mut::<bevy_ui::Node>() {
                    node.left = Val::Percent(fraction * 100.0);
                }
            })
            .on_drag_with(|world, entity, _delta| {
                let Some(range) = context::<Range>(world, entity).cloned() else {
                    return;
                };
                // Put the thumb exactly under the pointer: where the cursor sits along the track is the
                // value. Both the cursor and the track bounds are read in physical pixels, so this is
                // correct regardless of display scale (a pixel-delta mapping was not).
                let Some((left, width)) = track_bounds(world, entity) else {
                    return;
                };
                let Some(cursor) = window_cursor(world) else {
                    return;
                };
                if width <= 0.0 {
                    return;
                }
                let fraction = ((cursor.x - left) / width).clamp(0.0, 1.0);
                let next = range.min + fraction * (range.max - range.min);
                (range.on_change)(world, next);
            })
            .into()
    }
}

/// The thumb's track left edge and width in physical pixels (the same space as the cursor position).
fn track_bounds(world: &World, thumb: Entity) -> Option<(f32, f32)> {
    let track = world.get::<ChildOf>(thumb)?.parent();
    let computed = world.get::<ComputedNode>(track)?;
    let transform = world.get::<UiGlobalTransform>(track)?;
    let width = computed.size.x;
    Some((transform.translation.x - width / 2.0, width))
}

/// The primary window's cursor position in physical pixels.
fn window_cursor(world: &mut World) -> Option<Vec2> {
    let mut query = world.query_filtered::<&Window, With<PrimaryWindow>>();
    query
        .iter(world)
        .next()
        .and_then(|window| window.cursor_position())
}

/// The thumb/range fill fraction (0..1) from the shared [`Range`], read each render so both track the
/// controlled value.
fn fraction(world: &World, entity: Entity) -> f32 {
    context::<Range>(world, entity)
        .map(|range| ((range.value - range.min) / (range.max - range.min)).clamp(0.0, 1.0))
        .unwrap_or(0.0)
}

/// The slider root: flex row centered, 24px tall, 100% wide.
fn root_style() -> Style {
    Style::new().node(|node| {
        node.width = Val::Percent(100.0);
        node.height = Val::Px(size::STEP_600);
        node.display = bevy_ui::Display::Flex;
        node.align_items = AlignItems::Center;
    })
}

/// The track: 4px high, flex_grow fills the width, dark background, rounded. Holds the range and thumb,
/// positioned against it; it stays unclipped so the thumb can overhang.
fn track_style() -> Style {
    Style::new().background(color::scrim_dark).node(|node| {
        node.flex_grow = 1.0;
        node.height = Val::Px(size::STEP_100);
        node.border_radius = BorderRadius::all(Val::Px(radius::PILL));
        node.position_type = bevy_ui::PositionType::Relative;
    })
}

/// The range: anchored to the track's left, 100% height, fills rightward by the value fraction.
fn range_style() -> Style {
    Style::new().background(color::primary_base).node(|node| {
        node.position_type = bevy_ui::PositionType::Absolute;
        node.left = Val::Px(0.0);
        node.height = Val::Percent(100.0);
        node.border_radius = BorderRadius::all(Val::Px(radius::PILL));
    })
}

/// The 24px round thumb, centered on the 4px track (top `(4-24)/2 = -10`) and shifted left by half its
/// width so its centre — not its corner — sits on the value point.
fn thumb_style() -> Style {
    Style::new()
        .background(color::surface_canvas_base)
        .node(|node| {
            node.width = Val::Px(size::STEP_600);
            node.height = Val::Px(size::STEP_600);
            node.border_radius = BorderRadius::all(Val::Px(radius::L));
            node.position_type = bevy_ui::PositionType::Absolute;
            node.top = Val::Px(-10.0);
        })
        .translate(Vec2::new(-(size::STEP_600 / 2.0), 0.0))
}
