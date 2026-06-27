use bevy_ecs::hierarchy::Children;
use bevy_picking::prelude::Pickable;
use bevy_scene::{EntityScene, Scene, bsn, template_value};

use crate::component;
use bevy_ui::{
    AlignItems, BorderRadius, FlexDirection, JustifyContent, Node, Overflow, PositionType, UiRect,
    Val,
};

use crate::motion::Transform2d;
use crate::overlay::{Open, OverlayAction, OverlayContent, POPPER_ENTER, POPPER_EXIT, Portal};
use crate::style::Style;
use crate::theme::theme;
use crate::tokens::{radius, spacing};

pub fn dialog(open: bool, trigger: impl Scene, content: impl Scene) -> impl Scene {
    modal(open, true, trigger, content)
}

pub fn dialog_close() -> impl Scene {
    bsn! { component(OverlayAction::Close) }
}

// Without `Pickable::IGNORE` this full-screen centering container sits over the whole UI at the
// overlay z-index and swallows every click — a soft-lock; only the scrim child should capture input.
pub(crate) fn modal(
    open: bool,
    dismiss: bool,
    trigger: impl Scene,
    content: impl Scene,
) -> impl Scene {
    let scrim_close = dismiss.then_some(component(OverlayAction::Close));
    bsn! {
        Open({open})
        Children [
            (
                component(OverlayAction::Open)
                Children [ {EntityScene(trigger)} ]
            ),
            (
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(0.0),
                    left: Val::Px(0.0),
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                }
                Portal
                Pickable::IGNORE
                Children [
                    (
                        Node {
                            position_type: PositionType::Absolute,
                            top: Val::Px(0.0),
                            left: Val::Px(0.0),
                            width: Val::Percent(100.0),
                            height: Val::Percent(100.0),
                        }
                        {scrim_close}
                        {OverlayContent::animated(Transform2d::IDENTITY, Transform2d::IDENTITY)}
                        template_value(Style::new().background(theme().scrim_dark))
                    ),
                    (
                        {OverlayContent::animated(POPPER_ENTER, POPPER_EXIT)}
                        template_value(panel_style())
                        {content}
                    )
                ]
            )
        ]
    }
}

pub(crate) fn panel_style() -> Style {
    Style::new()
        .background(theme().surface_elevated.base)
        .text_color(theme().surface_elevated.on)
        .node(|node| {
            node.flex_direction = FlexDirection::Column;
            node.width = Val::Px(440.0);
            node.max_width = Val::Vw(90.0);
            node.padding = UiRect::all(Val::Px(spacing::XL));
            node.row_gap = Val::Px(spacing::XL);
            node.border_radius = BorderRadius::all(Val::Px(radius::M));
            node.overflow = Overflow::hidden();
        })
}
