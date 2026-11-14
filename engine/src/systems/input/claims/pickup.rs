use bevy::prelude::*;

use crate::core::render::Pointed;
use crate::core::render::world::decal::GroundDecal;
use crate::systems::input::claims::{Claimant, ClickClaim, Cursor};
use crate::systems::input::marks;
use crate::systems::interface::CursorShape;
use crate::systems::item::plates::{self, Named};
use crate::systems::item::{self, PickupRequest};
use crate::systems::player::session;

/// Takes a click on an item on the ground or on its plate. A plate the player asked for is
/// claimed apart, ahead of what stands under it: asking for names is looting.
pub enum PickupClaimant {
    AskedPlate,
    Item,
}

impl Claimant for PickupClaimant {
    fn claim(&self, world: &mut World, pointed: Option<&Pointed>) -> Option<ClickClaim> {
        if !session::is_alive(world) {
            return None;
        }
        pointed?;
        let item = match self {
            PickupClaimant::AskedPlate => plates::taken_by_plate(world, &[Named::ByKey]),
            PickupClaimant::Item => {
                plates::taken_by_plate(world, &[Named::ByPointer, Named::Unprompted])
                    .or_else(|| item::pickable_at(world, pointed?.ground?))
            }
        };
        item.map(ClickClaim::Pickup)
    }

    fn drive(&self, world: &mut World, claim: Option<ClickClaim>, start: bool) {
        if let Some(ClickClaim::Pickup(target)) = claim
            && start
        {
            world.write_message(PickupRequest { target });
        }
    }

    fn cursor(&self, world: &mut World, _claim: ClickClaim) -> Option<Cursor> {
        Some(Cursor::fixed(world, CursorShape::PickUp))
    }

    fn mark(&self, world: &mut World, claim: ClickClaim) -> Option<GroundDecal> {
        let ClickClaim::Pickup(target) = claim else {
            return None;
        };
        marks::under(world, target, marks::FRIENDLY)
    }
}
