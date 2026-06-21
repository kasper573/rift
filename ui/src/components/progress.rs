use bevy_ecs::hierarchy::ChildOf;
use bevy_ecs::prelude::*;
use bevy_ui::{BorderRadius, Node, Overflow, Val};

use crate::state::ancestor_with;
use crate::style::Style;
use crate::theme::color;
use crate::tokens::{radius, size};

#[derive(Component)]
pub struct ProgressFraction(pub f32);

#[derive(Component)]
pub struct ProgressIndicator;

pub fn progress(value: f32, max: f32) -> impl Bundle {
    let max = if max > 0.0 { max } else { 100.0 };
    let fraction = (value / max).clamp(0.0, 1.0);
    (
        Node::default(),
        ProgressFraction(fraction),
        Style::new()
            .background(color::surface_inset_base)
            .node(|node| {
                node.height = Val::Px(size::STEP_200);
                node.width = Val::Percent(100.0);
                node.border_radius = BorderRadius::all(Val::Px(radius::PILL));
                node.overflow = Overflow::clip_x();
            }),
    )
}

pub fn progress_indicator() -> impl Bundle {
    (
        Node::default(),
        ProgressIndicator,
        Style::new().background(color::primary_base).node(|node| {
            node.border_radius = BorderRadius::all(Val::Px(radius::PILL));
        }),
    )
}

pub(crate) fn sync_progress(
    fractions: Query<&ProgressFraction>,
    parents: Query<&ChildOf>,
    has_fraction: Query<(), With<ProgressFraction>>,
    mut indicators: Query<(Entity, &mut Node), With<ProgressIndicator>>,
) {
    for (entity, mut node) in &mut indicators {
        let fraction = ancestor_with::<ProgressFraction>(entity, &parents, &has_fraction)
            .and_then(|root| fractions.get(root).ok())
            .map_or(0.0, |fraction| fraction.0);
        node.height = Val::Percent(100.0);
        node.width = Val::Percent(fraction * 100.0);
    }
}
