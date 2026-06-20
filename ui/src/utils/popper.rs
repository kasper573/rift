//! Shared machinery for anchored floating overlays. Tooltips and popovers differ only in what opens
//! them (hover vs. click) and styling.

use bevy_view::{PortalKind, View, gate, portal};

use crate::controlled::when_open;
use crate::{Align, Placement, Side, floating};

macro_rules! placement_props {
    ($content:ident) => {
        impl $content {
            pub fn side(mut self, side: $crate::Side) -> $content {
                self.side = side;
                self
            }
            pub fn align(mut self, align: $crate::Align) -> $content {
                self.align = align;
                self
            }
            pub fn offset(mut self, offset: f32) -> $content {
                self.offset = offset;
                self
            }
        }
    };
}

pub(crate) use placement_props;

pub(crate) fn content(
    outlet: PortalKind,
    side: Side,
    align: Align,
    offset: f32,
    ignore_picking: bool,
    dismissable: bool,
    children: Vec<View>,
) -> View {
    let placement = Placement {
        side,
        align,
        offset,
    };
    let content = crate::exit_on_close(
        floating(placement, ignore_picking, dismissable, children),
        crate::POPPER_ENTER,
        crate::POPPER_EXIT,
    );
    gate(when_open, portal(outlet, content))
}
