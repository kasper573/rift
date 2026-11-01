use bevy::prelude::*;

use crate::core::render::Pointed;
use crate::systems::input::claims::{Claimant, ClickClaim, Cursor};
use crate::systems::interface::CursorShape;

pub struct DefaultClaimant;

impl Claimant for DefaultClaimant {
    fn claim(&self, _world: &mut World, _pointed: Option<&Pointed>) -> Option<ClickClaim> {
        Some(ClickClaim::Nothing)
    }

    fn drive(&self, _world: &mut World, _claim: Option<ClickClaim>, _start: bool) {}

    fn cursor(&self, _world: &mut World, _claim: ClickClaim) -> Option<Cursor> {
        Some(Cursor::Shaped(CursorShape::Default))
    }
}
