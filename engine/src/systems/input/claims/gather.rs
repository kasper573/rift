use bevy::prelude::*;

use crate::core::render::Pointed;
use crate::core::render::world::decal::GroundDecal;
use crate::systems::input::claims::{Claimant, ClickClaim, Cursor};
use crate::systems::input::marks;
use crate::systems::interface::CursorShape;
use crate::systems::player::session;
use crate::systems::resource::GatherRequest;
use crate::systems::resource::render::{outlook, pointed_at};

pub struct GatherClaimant;

impl Claimant for GatherClaimant {
    fn claim(&self, world: &mut World, pointed: Option<&Pointed>) -> Option<ClickClaim> {
        if !session::is_alive(world) {
            return None;
        }
        pointed_at(world, pointed?).map(ClickClaim::Gather)
    }

    fn drive(&self, world: &mut World, claim: Option<ClickClaim>, start: bool) {
        if let Some(ClickClaim::Gather(target)) = claim
            && start
        {
            world.write_message(GatherRequest { target });
        }
    }

    fn cursor(&self, world: &mut World, claim: ClickClaim) -> Option<Cursor> {
        let ClickClaim::Gather(target) = claim else {
            return None;
        };
        Some(Cursor::Shaped(if workable(world, target) {
            CursorShape::Gather
        } else {
            CursorShape::Default
        }))
    }

    fn mark(&self, world: &mut World, claim: ClickClaim) -> Option<GroundDecal> {
        let ClickClaim::Gather(target) = claim else {
            return None;
        };
        workable(world, target)
            .then(|| marks::under(world, target, marks::FRIENDLY))
            .flatten()
    }
}

fn workable(world: &mut World, resource: Entity) -> bool {
    session::me(world)
        .and_then(|me| outlook(world, me, resource))
        .is_some_and(|outlook| outlook.workable())
}
