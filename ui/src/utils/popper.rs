//! `popper`: the shared machinery behind anchored floating overlays. A tooltip and a popover are the
//! same thing — content that collision-positions against a registered trigger anchor, mounts into an
//! outlet while open, and eases in — and differ only in what opens them (the tooltip on hover, the
//! popover on click) and their surface styling. That common content/anchoring/portaling lives here so
//! the components don't re-implement it.

use bevy_view::{PortalKind, View, gate, portal};

use crate::controlled::when_open;
use crate::{Align, Placement, Side, floating};

/// The side/align/offset placement controls every popper content exposes — generates the builder
/// methods over a struct that holds `side`, `align` and `offset` fields.
macro_rules! placement_props {
    ($content:ident) => {
        impl $content {
            /// The side of the trigger the content prefers.
            pub fn side(mut self, side: $crate::Side) -> $content {
                self.side = side;
                self
            }
            /// How the content aligns along the trigger's cross axis.
            pub fn align(mut self, align: $crate::Align) -> $content {
                self.align = align;
                self
            }
            /// The gap between the trigger and the content, in logical pixels.
            pub fn offset(mut self, offset: f32) -> $content {
                self.offset = offset;
                self
            }
        }
    };
}

pub(crate) use placement_props;

/// Builds an overlay's floating content: shown only while open (`gate`), portaled to `outlet`, placed
/// against its anchor by `side`/`align`/`offset`, and faded + scaled in/out by the shared popper
/// animation. It imposes NO appearance or bounds — `children` is the whole content, so the app composes
/// whatever it wants (e.g. a `Card`, or bare text). `ignore_picking` makes it click-through (tooltips)
/// and `dismissable` closes it on a press outside (popovers).
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
