use bevy::prelude::*;

use crate::core::content::Content;
use crate::core::render::Pointed;
use crate::core::render::world::decal::GroundDecal;
use crate::systems::input::claims::{Claimant, ClickClaim, Cursor};
use crate::systems::input::marks;
use crate::systems::player::session;
use crate::systems::resource::render::pointed_at;
use crate::systems::resource::{GatherRequest, Resource};

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
        let content = world.resource::<Content>();
        let kind = world.get::<Resource>(target)?.def.get(content).tool;
        Some(Cursor::Shaped(kind.get(content).cursor))
    }

    fn mark(&self, world: &mut World, claim: ClickClaim) -> Option<GroundDecal> {
        let ClickClaim::Gather(target) = claim else {
            return None;
        };
        marks::under(world, target, marks::FRIENDLY)
    }
}
