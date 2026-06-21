use bevy_ecs::bundle::Bundle;
use bevy_ecs::children;
use bevy_picking::prelude::Pickable;
use bevy_ui::{Node, PositionType, Val};

use crate::components::dialog::{full_screen_center, panel_style};
use crate::motion::Transform2d;
use crate::overlay::{Open, OverlayAction, OverlayContent, POPPER_ENTER, POPPER_EXIT, Portal};
use crate::style::Style;
use crate::theme::color;

/// Like [`dialog`](super::dialog), but the scrim does **not** dismiss — an alert demands an explicit
/// choice, so only the buttons in `content` (wrapped in [`alert_dialog_cancel`]/[`alert_dialog_action`])
/// close it.
pub fn alert_dialog(open: bool, trigger: impl Bundle, content: impl Bundle) -> impl Bundle {
    (
        Node::default(),
        Open(open),
        children![
            (alert_dialog_trigger(), children![trigger]),
            (
                alert_dialog_modal(),
                children![alert_dialog_scrim(), (alert_dialog_content(), content)],
            ),
        ],
    )
}

pub fn alert_dialog_cancel() -> impl Bundle {
    (Node::default(), OverlayAction::Close)
}

pub fn alert_dialog_action() -> impl Bundle {
    (Node::default(), OverlayAction::Close)
}

pub(crate) fn alert_dialog_trigger() -> impl Bundle {
    (Node::default(), OverlayAction::Open)
}

pub(crate) fn alert_dialog_modal() -> impl Bundle {
    (full_screen_center(), Portal, Pickable::IGNORE)
}

// No `OverlayAction::Close`: clicking outside an alert must not dismiss it.
pub(crate) fn alert_dialog_scrim() -> impl Bundle {
    (
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(0.0),
            left: Val::Px(0.0),
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..Node::default()
        },
        OverlayContent::animated(Transform2d::IDENTITY, Transform2d::IDENTITY),
        Style::new().background(color::scrim_dark),
    )
}

pub(crate) fn alert_dialog_content() -> impl Bundle {
    (
        Node::default(),
        OverlayContent::animated(POPPER_ENTER, POPPER_EXIT),
        panel_style(),
    )
}
