use bevy::prelude::*;

use crate::core::render::Pointed;
use crate::systems::input::claims::{Claimant, ClickClaim, Cursor};
use crate::systems::interface::CursorShape;
use crate::systems::player::session;

pub struct LockedClaimant;

impl Claimant for LockedClaimant {
    fn claim(&self, world: &mut World, _pointed: Option<&Pointed>) -> Option<ClickClaim> {
        session::is_locked(world).then_some(ClickClaim::Locked)
    }

    fn drive(&self, _world: &mut World, _claim: Option<ClickClaim>, _start: bool) {}

    fn cursor(&self, world: &mut World, _claim: ClickClaim) -> Option<Cursor> {
        Some(Cursor::fixed(world, CursorShape::Default))
    }
}
