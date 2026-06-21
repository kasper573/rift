use bevy_ecs::bundle::Bundle;
use bevy_picking::prelude::Pickable;
use bevy_ui::{
    AlignItems, BorderRadius, FlexDirection, JustifyContent, Node, Overflow, PositionType, UiRect,
    Val,
};

use crate::motion::Transform2d;
use crate::overlay::{Open, OverlayAction, OverlayContent, POPPER_ENTER, POPPER_EXIT, Portal};
use crate::style::Style;
use crate::theme::color;
use crate::tokens::{radius, spacing};

pub fn dialog(open: bool) -> impl Bundle {
    (Node::default(), Open(open))
}

pub fn dialog_trigger() -> impl Bundle {
    (Node::default(), OverlayAction::Open)
}

// The modal is only a full-screen centering container; it must not capture input itself (the scrim
// child does that). Without `Pickable::IGNORE` it sits at the overlay z-index over the whole UI and
// swallows every click — a soft-lock.
pub fn dialog_modal() -> impl Bundle {
    (full_screen_center(), Portal, Pickable::IGNORE)
}

pub fn dialog_scrim() -> impl Bundle {
    (
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(0.0),
            left: Val::Px(0.0),
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..Node::default()
        },
        OverlayAction::Close,
        OverlayContent::animated(Transform2d::IDENTITY, Transform2d::IDENTITY),
        Style::new().background(color::scrim_dark),
    )
}

pub fn dialog_content() -> impl Bundle {
    (
        Node::default(),
        OverlayContent::animated(POPPER_ENTER, POPPER_EXIT),
        panel_style(),
    )
}

pub fn dialog_close() -> impl Bundle {
    (Node::default(), OverlayAction::Close)
}

pub(crate) fn full_screen_center() -> Node {
    Node {
        position_type: PositionType::Absolute,
        top: Val::Px(0.0),
        left: Val::Px(0.0),
        width: Val::Percent(100.0),
        height: Val::Percent(100.0),
        align_items: AlignItems::Center,
        justify_content: JustifyContent::Center,
        ..Node::default()
    }
}

pub(crate) fn panel_style() -> Style {
    Style::new()
        .background(color::surface_elevated.base)
        .text_color(color::surface_elevated.on)
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
