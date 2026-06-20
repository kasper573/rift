use std::sync::Arc;

use bevy_ecs::prelude::*;
use bevy_math::Vec2;
use bevy_ui::{BorderRadius, UiRect, Val};
use bevy_view::{View, button, node, provide};

use crate::controlled::{OnChange, noop};
use crate::motion::transition::STANDARD_ENTER;
use crate::motion::{Easing, Timing};
use crate::recipe::{Style, Styled};
use crate::theme::color;
use crate::tokens::{radius, size};

#[derive(Default)]
pub struct Switch {
    checked: bool,
    on_checked_change: Option<OnChange<bool>>,
    children: Vec<View>,
}

impl Switch {
    pub fn checked(mut self, checked: bool) -> Switch {
        self.checked = checked;
        self
    }

    pub fn on_checked_change<F>(mut self, handler: F) -> Switch
    where
        F: Fn(&mut World, bool) + Send + Sync + 'static,
    {
        self.on_checked_change = Some(Arc::new(handler));
        self
    }
}

children_builder!(Switch);

#[derive(Component, Clone, Copy)]
struct SwitchOn(bool);

#[derive(Default)]
pub struct SwitchThumb;

impl From<Switch> for View {
    fn from(switch: Switch) -> View {
        let checked = switch.checked;
        let on_change = switch.on_checked_change.unwrap_or_else(noop);
        button()
            .style(track_style(checked))
            .bind(provide(SwitchOn(checked)))
            .on_click(move |world| on_change(world, !checked))
            .children(switch.children)
            .into()
    }
}

impl From<SwitchThumb> for View {
    fn from(_: SwitchThumb) -> View {
        node()
            .attr(|entity| {
                let id = entity.id();
                let on = entity.world_scope(|world| {
                    bevy_view::context::<SwitchOn>(world, id)
                        .map(|ctx| ctx.0)
                        .unwrap_or(false)
                });
                thumb_style(on).apply(entity);
            })
            .into()
    }
}

fn track_style(checked: bool) -> Style {
    let bg = if checked {
        color::primary_base
    } else {
        color::surface_elevated_base
    };
    Style::new()
        .node(move |node| {
            node.width = Val::Px(52.0);
            node.height = Val::Px(32.0);
            node.border = UiRect::all(Val::Px(if checked { 0.0 } else { 2.0 }));
            node.border_radius = BorderRadius::all(Val::Px(radius::PILL));
            node.position_type = bevy_ui::PositionType::Relative;
        })
        .background(bg)
        .border_color(color::surface_canvas_border)
        .transition(STANDARD_ENTER)
}

fn thumb_style(on: bool) -> Style {
    let inset = if on { 4.0 } else { 2.0 };
    Style::new()
        .node(move |node| {
            node.width = Val::Px(size::STEP_600);
            node.height = Val::Px(size::STEP_600);
            node.position_type = bevy_ui::PositionType::Absolute;
            node.top = Val::Px(inset);
            node.left = Val::Px(inset);
            node.border_radius = BorderRadius::all(Val::Px(radius::PILL));
        })
        .background(if on {
            color::surface_floating_base
        } else {
            color::surface_canvas_border
        })
        .translate(Vec2::new(if on { 20.0 } else { 0.0 }, 0.0))
        .transition(Timing::new(150, Easing::Standard))
}
