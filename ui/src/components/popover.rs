use bevy_ecs::bundle::Bundle;
use bevy_ui::{GlobalZIndex, Node, PositionType};

use crate::overlay::{Dismissable, Open, OverlayAction, OverlayContent, POPPER_ENTER, POPPER_EXIT};
use crate::place::Placement;
use crate::{Align, Side};

pub(crate) const ANCHORED_Z: i32 = 900;

// No `Node` (like `tooltip`): the trigger/anchor node carries it so the floating content anchors to
// the trigger, and the consumer positions that node.
pub fn popover(open: bool) -> impl Bundle {
    (Open(open), Dismissable)
}

pub fn popover_trigger() -> impl Bundle {
    (Node::default(), OverlayAction::Toggle)
}

pub fn popover_content(side: Side, align: Align, offset: f32) -> impl Bundle {
    (
        Node {
            position_type: PositionType::Absolute,
            ..Node::default()
        },
        GlobalZIndex(ANCHORED_Z),
        Placement {
            side,
            align,
            offset,
        },
        OverlayContent::animated(POPPER_ENTER, POPPER_EXIT),
    )
}

pub fn popover_close() -> impl Bundle {
    (Node::default(), OverlayAction::Close)
}
