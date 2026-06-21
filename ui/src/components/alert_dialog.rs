use bevy_ecs::bundle::Bundle;
use bevy_picking::prelude::Pickable;
use bevy_ui::{Node, PositionType, Val};

use crate::components::dialog::{full_screen_center, panel_style};
use crate::motion::Transform2d;
use crate::overlay::{Open, OverlayAction, OverlayContent, POPPER_ENTER, POPPER_EXIT, Portal};
use crate::style::Style;
use crate::theme::color;

pub fn alert_dialog(open: bool) -> impl Bundle {
    (Node::default(), Open(open))
}

pub fn alert_dialog_trigger() -> impl Bundle {
    (Node::default(), OverlayAction::Open)
}

pub fn alert_dialog_modal() -> impl Bundle {
    (full_screen_center(), Portal, Pickable::IGNORE)
}

pub fn alert_dialog_scrim() -> impl Bundle {
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

pub fn alert_dialog_content() -> impl Bundle {
    (
        Node::default(),
        OverlayContent::animated(POPPER_ENTER, POPPER_EXIT),
        panel_style(),
    )
}

pub fn alert_dialog_cancel() -> impl Bundle {
    (Node::default(), OverlayAction::Close)
}

pub fn alert_dialog_action() -> impl Bundle {
    (Node::default(), OverlayAction::Close)
}
