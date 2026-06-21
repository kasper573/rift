use std::time::Duration;

use bevy_picking::prelude::Pickable;
use bevy_scene::{Scene, bsn};
use bevy_ui::{GlobalZIndex, Node, PositionType};

use crate::component;
use crate::components::popover::ANCHORED_Z;
use crate::overlay::{Open, OverlayContent, POPPER_ENTER, POPPER_EXIT, TooltipTimer};
use crate::place::Placement;
use crate::{Align, Side};

const DELAY: Duration = Duration::from_millis(400);
const SKIP_DELAY: Duration = Duration::from_millis(300);

// No `Node`: the trigger node carries this so the floating content anchors directly to the trigger,
// which matters when the trigger is absolutely positioned (e.g. a draggable HUD widget).
pub fn tooltip(open: bool) -> impl Scene {
    bsn! {
        Open({open})
        component(TooltipTimer::new(DELAY, SKIP_DELAY))
    }
}

pub fn tooltip_content(side: Side, align: Align, offset: f32) -> impl Scene {
    bsn! {
        Node { position_type: PositionType::Absolute }
        GlobalZIndex({ANCHORED_Z})
        Placement { side: {side}, align: {align}, offset: {offset} }
        {OverlayContent::animated(POPPER_ENTER, POPPER_EXIT)}
        Pickable::IGNORE
    }
}
