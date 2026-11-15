use bevy::prelude::*;

use crate::core::render::Pointed;
use crate::core::render::world::decal::GroundDecal;
use crate::systems::actor;
use crate::systems::input::claims::{Claimant, ClickClaim, Cursor};
use crate::systems::input::marks;
use crate::systems::interface::CursorShape;
use crate::systems::mount;
use crate::systems::mount::seat::{self, MountRequest};
use crate::systems::player::session;

/// Takes a click on a mount with a seat free, which asks its driver for the seat.
pub struct BoardClaimant;

impl Claimant for BoardClaimant {
    fn claim(&self, world: &mut World, pointed: Option<&Pointed>) -> Option<ClickClaim> {
        let me = session::me(world)?;
        if !session::is_alive(world) || mount::rides(world, me) {
            return None;
        }
        let free: Vec<Entity> = seat::drivers_with_room(world);
        actor::target_at(world, pointed?, Some(me), |_, entity| {
            free.contains(&entity)
        })
        .map(ClickClaim::Board)
    }

    fn drive(&self, world: &mut World, claim: Option<ClickClaim>, start: bool) {
        if let Some(ClickClaim::Board(driver)) = claim
            && start
        {
            world.write_message(MountRequest::AskSeat { driver });
        }
    }

    fn cursor(&self, world: &mut World, claim: ClickClaim) -> Option<Cursor> {
        matches!(claim, ClickClaim::Board(_)).then(|| Cursor::fixed(world, CursorShape::Board))
    }

    fn mark(&self, world: &mut World, claim: ClickClaim) -> Option<GroundDecal> {
        let ClickClaim::Board(driver) = claim else {
            return None;
        };
        marks::under(world, driver, marks::FRIENDLY)
    }
}
