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

const DIALOG_OUTLET: PortalKind = PortalKind(0x0d1a_0000_0000_0001);

#[derive(Default)]
pub struct Dialog {
    open: bool,
    on_open_change: Option<OnChange<bool>>,
    children: Vec<View>,
}

impl Dialog {
    pub fn open(mut self, open: bool) -> Dialog {
        self.open = open;
        self
    }

    pub fn on_open_change<F>(mut self, handler: F) -> Dialog
    where
        F: Fn(&mut World, bool) + Send + Sync + 'static,
    {
        self.on_open_change = Some(Arc::new(handler));
        self
    }
}

children_builder!(Dialog);

#[derive(Default)]
pub struct DialogTrigger {
    children: Vec<View>,
}

children_builder!(DialogTrigger);

#[derive(Default)]
pub struct DialogOverlay {
    children: Vec<View>,
}

children_builder!(DialogOverlay);

#[derive(Default)]
pub struct DialogContent {
    children: Vec<View>,
}

children_builder!(DialogContent);

#[derive(Default)]
pub struct DialogTitle {
    children: Vec<View>,
}

children_builder!(DialogTitle);

#[derive(Default)]
pub struct DialogDescription {
    children: Vec<View>,
}

children_builder!(DialogDescription);

#[derive(Default)]
pub struct DialogClose {
    children: Vec<View>,
}

children_builder!(DialogClose);

/// Where dialogs render — place it last to paint above the rest of the UI.
#[derive(Default)]
pub struct DialogOutlet;

fn fill(entity: &mut bevy_ecs::world::EntityWorldMut) {
    if let Some(mut node) = entity.get_mut::<Node>() {
        node.position_type = PositionType::Absolute;
        node.width = Val::Percent(100.0);
        node.height = Val::Percent(100.0);
    }
}

impl From<Dialog> for View {
    fn from(dialog: Dialog) -> View {
        overlay_root(
            dialog.open,
            dialog.on_open_change.unwrap_or_else(noop),
            dialog.children,
        )
    }
}

impl From<DialogTrigger> for View {
    fn from(trigger: DialogTrigger) -> View {
        node()
            .on_click_with(drive(true))
            .children(trigger.children)
            .into()
    }
}

impl From<DialogOverlay> for View {
    fn from(overlay: DialogOverlay) -> View {
        let style = Style::new().background(color::scrim_dark);
        let scrim = crate::exit_on_close(
            node()
                .attr(fill)
                .style(style)
                .on_click_with(close_overlay)
                .children(overlay.children),
            Transform2d::IDENTITY,
            Transform2d::IDENTITY,
        );
        gate(when_open, portal(DIALOG_OUTLET, scrim))
    }
}

impl From<DialogContent> for View {
    fn from(content: DialogContent) -> View {
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
        // Opacity + scale (enter/exit) owned by `exit_on_close`.
        let panel = crate::exit_on_close(
            node().style(style).children(content.children),
            crate::POPPER_ENTER,
            crate::POPPER_EXIT,
        );
        gate(when_open, portal(DIALOG_OUTLET, panel))
    }
}

impl From<DialogTitle> for View {
    fn from(title: DialogTitle) -> View {
        node().children(title.children).into()
    }
}

impl From<DialogDescription> for View {
    fn from(description: DialogDescription) -> View {
        node().children(description.children).into()
    }
}

impl From<DialogClose> for View {
    fn from(close: DialogClose) -> View {
        node()
            .on_click_with(close_overlay)
            .children(close.children)
            .into()
    }
}

impl From<DialogOutlet> for View {
    fn from(_: DialogOutlet) -> View {
        outlet(DIALOG_OUTLET)
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
