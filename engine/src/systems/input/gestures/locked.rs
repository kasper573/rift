use bevy::prelude::*;

use crate::core::math::Pos;
use crate::core::units::Meters;
use crate::systems::input::gestures::{ClickClaim, Cursor, Gesture};
use crate::systems::interface::CursorShape;
use crate::systems::player::session;

pub struct LockedGesture;

impl Gesture for LockedGesture {
    fn claim(&self, world: &mut World, _tile: Option<Pos<Meters>>) -> Option<ClickClaim> {
        session::is_locked(world).then_some(ClickClaim::Locked)
    }

    fn drive(&self, _world: &mut World, _claim: Option<ClickClaim>, _start: bool) {}

    fn cursor(&self, _world: &mut World, _claim: ClickClaim) -> Option<Cursor> {
        Some(Cursor::Shaped(CursorShape::Default))
    }
}
