use bevy_ui::{BorderRadius, Node, Val};
use bevy_view::{View, context, node, provide};

use crate::recipe::{Style, Styled};
use crate::theme::color;
use crate::tokens::{radius, size};

#[derive(Clone, Copy)]
struct Fraction(f32);

#[derive(Default)]
pub struct Progress {
    value: f32,
    max: f32,
    children: Vec<View>,
}

impl Progress {
    pub fn value(mut self, value: f32) -> Progress {
        self.value = value;
        self
    }
    pub fn max(mut self, max: f32) -> Progress {
        self.max = max;
        self
    }
}

children_builder!(Progress);

#[derive(Default)]
pub struct ProgressIndicator;

impl From<Progress> for View {
    fn from(progress: Progress) -> View {
        let max = if progress.max > 0.0 {
            progress.max
        } else {
            100.0
        };
        let fraction = (progress.value / max).clamp(0.0, 1.0);
        let style = track_style();
        node()
            .style(style)
            .bind(provide(Fraction(fraction)))
            .children(progress.children)
            .into()
    }
}

impl From<ProgressIndicator> for View {
    fn from(_: ProgressIndicator) -> View {
        node()
            .style(indicator_style())
            .attr(|entity| {
                let id = entity.id();
                let fraction =
                    entity.world_scope(|world| context::<Fraction>(world, id).map_or(0.0, |f| f.0));
                if let Some(mut node) = entity.get_mut::<Node>() {
                    node.height = Val::Percent(100.0);
                    node.width = Val::Percent(fraction * 100.0);
                }
            })
            .into()
    }
}

fn track_style() -> Style {
    Style::new()
        .background(color::surface_inset_base)
        .node(|node| {
            node.height = Val::Px(size::STEP_200);
            node.width = Val::Percent(100.0);
            node.border_radius = BorderRadius::all(Val::Px(radius::PILL));
            node.overflow = bevy_ui::Overflow::clip_x();
        })
}

fn indicator_style() -> Style {
    Style::new().background(color::primary_base).node(|node| {
        node.border_radius = BorderRadius::all(Val::Px(radius::PILL));
    })
}
