use std::sync::Arc;

use bevy_ecs::prelude::*;
use bevy_view::{PortalKind, View, node};

use crate::controlled::{OnChange, flip, noop};
use crate::{Align, Side, close_overlay, overlay_root, register_anchor};

const POPOVER_OUTLET: PortalKind = PortalKind(0xb0_0000_0000_0090);

#[derive(Default)]
pub struct Popover {
    open: bool,
    on_open_change: Option<OnChange<bool>>,
    children: Vec<View>,
}

impl Popover {
    pub fn open(mut self, open: bool) -> Popover {
        self.open = open;
        self
    }

    pub fn on_open_change<F>(mut self, handler: F) -> Popover
    where
        F: Fn(&mut World, bool) + Send + Sync + 'static,
    {
        self.on_open_change = Some(Arc::new(handler));
        self
    }
}

#[derive(Default)]
pub struct PopoverTrigger {
    children: Vec<View>,
}

#[derive(Default)]
pub struct PopoverContent {
    side: Side,
    align: Align,
    offset: f32,
    children: Vec<View>,
}

#[derive(Default)]
pub struct PopoverClose {
    children: Vec<View>,
}

#[derive(Default)]
pub struct PopoverOutlet;

children_builder!(Popover);
children_builder!(PopoverTrigger);
children_builder!(PopoverContent);
children_builder!(PopoverClose);

crate::popper::placement_props!(PopoverContent);

impl From<Popover> for View {
    fn from(popover: Popover) -> View {
        overlay_root(
            popover.open,
            popover.on_open_change.unwrap_or_else(noop),
            popover.children,
        )
    }
}

impl From<PopoverTrigger> for View {
    fn from(trigger: PopoverTrigger) -> View {
        node()
            .on_mount_with(register_anchor)
            .on_click_with(flip)
            .children(trigger.children)
            .into()
    }
}

impl From<PopoverContent> for View {
    fn from(content: PopoverContent) -> View {
        // No appearance; the popover floats composed content (e.g. a `Card`). Dismissable on outside press.
        crate::popper::content(
            POPOVER_OUTLET,
            content.side,
            content.align,
            content.offset,
            false,
            true,
            content.children,
        )
    }
}

impl From<PopoverClose> for View {
    fn from(close: PopoverClose) -> View {
        node()
            .on_click_with(close_overlay)
            .children(close.children)
            .into()
    }
}

impl From<PopoverOutlet> for View {
    fn from(_: PopoverOutlet) -> View {
        crate::overlay_outlet(POPOVER_OUTLET).into()
    }
}
