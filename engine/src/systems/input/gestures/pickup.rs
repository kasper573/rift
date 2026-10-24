use bevy::prelude::*;

use crate::core::math::Pos;
use crate::core::units::Meters;
use crate::systems::input::gestures::{ClickClaim, Cursor, Gesture};
use crate::systems::interface::CursorShape;
use crate::systems::item::{self, PickupRequest};
use crate::systems::player::session;

pub struct PickupGesture;

impl Gesture for PickupGesture {
    fn claim(&self, world: &mut World, tile: Option<Pos<Meters>>) -> Option<ClickClaim> {
        if !session::is_alive(world) {
            return None;
        }
        item::pickable_at(world, tile?).map(ClickClaim::Pickup)
    }

    fn drive(&self, world: &mut World, claim: Option<ClickClaim>, start: bool) {
        if let Some(ClickClaim::Pickup(target)) = claim
            && start
        {
            world.write_message(PickupRequest { target });
        }
    }

    fn cursor(&self, _world: &mut World, _claim: ClickClaim) -> Option<Cursor> {
        Some(Cursor::Shaped(CursorShape::PickUp))
    }
}
