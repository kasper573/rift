//! `AlertDialog`: a modal confirmation. Like [`Dialog`](crate::Dialog) — controlled `open` +
//! `on_open_change` — but the backdrop does not dismiss it; only `Cancel` or `Action` close it.

use std::sync::Arc;

use bevy_ecs::prelude::*;
use bevy_picking::prelude::Pickable;
use bevy_ui::{
    AlignItems, BorderRadius, FlexDirection, JustifyContent, Node, Overflow, PositionType, UiRect,
    Val,
};
use bevy_view::{PortalKind, View, gate, node, outlet, portal};

use crate::controlled::{OnChange, drive, noop, when_open};
use crate::motion::Transform2d;
use crate::{
    close_overlay, overlay_root,
    recipe::{Style, Styled},
    theme::color,
    tokens::{radius, spacing},
};

const ALERT_OUTLET: PortalKind = PortalKind(0x0a1e_0000_0000_0001);

#[derive(Default)]
pub struct AlertDialog {
    open: bool,
    on_open_change: Option<OnChange<bool>>,
    children: Vec<View>,
}

impl AlertDialog {
    pub fn open(mut self, open: bool) -> AlertDialog {
        self.open = open;
        self
    }

    pub fn on_open_change<F>(mut self, handler: F) -> AlertDialog
    where
        F: Fn(&mut World, bool) + Send + Sync + 'static,
    {
        self.on_open_change = Some(Arc::new(handler));
        self
    }
}

children_builder!(AlertDialog);

/// Opens the alert when clicked.
#[derive(Default)]
pub struct AlertDialogTrigger {
    children: Vec<View>,
}

children_builder!(AlertDialogTrigger);

/// The non-dismissing backdrop.
#[derive(Default)]
pub struct AlertDialogOverlay {
    children: Vec<View>,
}

children_builder!(AlertDialogOverlay);

/// The alert panel, centered over the backdrop.
#[derive(Default)]
pub struct AlertDialogContent {
    children: Vec<View>,
}

children_builder!(AlertDialogContent);

#[derive(Default)]
pub struct AlertDialogTitle {
    children: Vec<View>,
}

children_builder!(AlertDialogTitle);

#[derive(Default)]
pub struct AlertDialogDescription {
    children: Vec<View>,
}

children_builder!(AlertDialogDescription);

/// Dismisses the alert without acting.
#[derive(Default)]
pub struct AlertDialogCancel {
    children: Vec<View>,
}

children_builder!(AlertDialogCancel);

/// Confirms and closes the alert.
#[derive(Default)]
pub struct AlertDialogAction {
    children: Vec<View>,
}

children_builder!(AlertDialogAction);

/// Where alert dialogs render.
#[derive(Default)]
pub struct AlertDialogOutlet;

impl From<AlertDialog> for View {
    fn from(alert: AlertDialog) -> View {
        overlay_root(
            alert.open,
            alert.on_open_change.unwrap_or_else(noop),
            alert.children,
        )
    }
}

impl From<AlertDialogTrigger> for View {
    fn from(trigger: AlertDialogTrigger) -> View {
        node()
            .on_click_with(drive(true))
            .children(trigger.children)
            .into()
    }
}

impl From<AlertDialogOverlay> for View {
    fn from(overlay: AlertDialogOverlay) -> View {
        // The backdrop just paints; its fade in/out is owned by `exit_on_close` (no transform).
        let style = Style::new().background(color::scrim_dark);
        let scrim = crate::exit_on_close(
            node()
                .attr(|entity| {
                    if let Some(mut node) = entity.get_mut::<Node>() {
                        node.position_type = PositionType::Absolute;
                        node.width = Val::Percent(100.0);
                        node.height = Val::Percent(100.0);
                    }
                })
                .style(style)
                .children(overlay.children),
            Transform2d::IDENTITY,
            Transform2d::IDENTITY,
        );
        gate(when_open, portal(ALERT_OUTLET, scrim))
    }
}

impl From<AlertDialogContent> for View {
    fn from(content: AlertDialogContent) -> View {
        let style = Style::new()
            .background(color::surface_elevated_base)
            .text_color(color::surface_elevated_on)
            .node(|node| {
                node.flex_direction = FlexDirection::Column;
                node.width = Val::Px(440.0);
                node.max_width = Val::Vw(90.0);
                node.padding = UiRect::all(Val::Px(spacing::XL));
                node.row_gap = Val::Px(spacing::XL);
                node.border_radius = BorderRadius::all(Val::Px(radius::M));
                node.overflow = Overflow::hidden();
            });
        // Opacity + scale (enter and exit) are owned by `exit_on_close`.
        let panel = crate::exit_on_close(
            node().style(style).children(content.children),
            crate::POPPER_ENTER,
            crate::POPPER_EXIT,
        );
        gate(when_open, portal(ALERT_OUTLET, panel))
    }
}

impl From<AlertDialogTitle> for View {
    fn from(title: AlertDialogTitle) -> View {
        node().children(title.children).into()
    }
}

impl From<AlertDialogDescription> for View {
    fn from(description: AlertDialogDescription) -> View {
        node().children(description.children).into()
    }
}

impl From<AlertDialogCancel> for View {
    fn from(cancel: AlertDialogCancel) -> View {
        node()
            .on_click_with(close_overlay)
            .children(cancel.children)
            .into()
    }
}

impl From<AlertDialogAction> for View {
    fn from(action: AlertDialogAction) -> View {
        node()
            .on_click_with(close_overlay)
            .children(action.children)
            .into()
    }
}

impl From<AlertDialogOutlet> for View {
    fn from(_: AlertDialogOutlet) -> View {
        outlet(ALERT_OUTLET)
            .attr(|entity| {
                if let Some(mut node) = entity.get_mut::<Node>() {
                    node.position_type = PositionType::Absolute;
                    node.width = Val::Percent(100.0);
                    node.height = Val::Percent(100.0);
                    node.align_items = AlignItems::Center;
                    node.justify_content = JustifyContent::Center;
                }
            })
            .insert(Pickable::IGNORE)
            .into()
    }
}
