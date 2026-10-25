use bevy::prelude::*;

use crate::core::render::Pointed;
use crate::core::render::world::decal::GroundDecal;
use crate::systems::input::gestures::{ClickClaim, Cursor, Gesture};
use crate::systems::input::marks;
use crate::systems::interface::CursorShape;
use crate::systems::item::{self, PickupRequest};
use crate::systems::player::session;

pub struct PickupGesture;

impl Gesture for PickupGesture {
    fn claim(&self, world: &mut World, pointed: Option<&Pointed>) -> Option<ClickClaim> {
        if !session::is_alive(world) {
            return None;
        }
        item::pickable_at(world, pointed?.ground?).map(ClickClaim::Pickup)
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

    fn mark(&self, world: &mut World, claim: ClickClaim) -> Option<GroundDecal> {
        let ClickClaim::Pickup(target) = claim else {
            return None;
        };
        marks::under(world, target, marks::FRIENDLY)
    }
}
