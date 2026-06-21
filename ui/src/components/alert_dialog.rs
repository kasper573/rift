use bevy_scene::{Scene, bsn};

use crate::component;
use crate::components::dialog::modal;
use crate::overlay::OverlayAction;

/// Like [`dialog`](super::dialog), but the scrim does **not** dismiss — an alert demands an explicit
/// choice, so only the buttons in `content` (wrapped in [`alert_dialog_cancel`]/[`alert_dialog_action`])
/// close it.
pub fn alert_dialog(open: bool, trigger: impl Scene, content: impl Scene) -> impl Scene {
    modal(open, false, trigger, content)
}

pub fn alert_dialog_cancel() -> impl Scene {
    bsn! { component(OverlayAction::Close) }
}

pub fn alert_dialog_action() -> impl Scene {
    bsn! { component(OverlayAction::Close) }
}
