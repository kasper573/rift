use bevy_scene::{Scene, bsn};
use bevy_ui::{GlobalZIndex, Node, OverrideClip, PositionType};

use crate::component;
use crate::overlay::{Dismissable, Open, OverlayAction, OverlayContent, POPPER_ENTER, POPPER_EXIT};
use crate::place::Placement;
use crate::tokens::layer;
use crate::{Align, Side};

pub fn popover(open: bool) -> impl Scene {
    bsn! {
        Open({open})
        Dismissable
    }
}

pub fn popover_trigger() -> impl Scene {
    bsn! {
        component(OverlayAction::Toggle)
    }
}

pub fn popover_content(side: Side, align: Align, offset: f32) -> impl Scene {
    bsn! {
        Node { position_type: PositionType::Absolute }
        GlobalZIndex({layer::ANCHORED})
        OverrideClip
        Placement { side: {side}, align: {align}, offset: {offset} }
        {OverlayContent::animated(POPPER_ENTER, POPPER_EXIT)}
    }
}

pub fn popover_close() -> impl Scene {
    bsn! {
        component(OverlayAction::Close)
    }
}
