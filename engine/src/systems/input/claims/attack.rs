use bevy::prelude::*;

use crate::core::render::Pointed;
use crate::core::render::world::decal::GroundDecal;
use crate::systems::combat::{self, AttackRequest};
use crate::systems::input::claims::{Claimant, ClickClaim, Cursor};
use crate::systems::input::marks;
use crate::systems::interface::CursorShape;
use crate::systems::player::session;

pub struct AttackClaimant;

impl Claimant for AttackClaimant {
    fn claim(&self, world: &mut World, pointed: Option<&Pointed>) -> Option<ClickClaim> {
        if !session::is_alive(world) {
            return None;
        }
        combat::enemy_at(world, pointed?).map(ClickClaim::Attack)
    }

    fn drive(&self, world: &mut World, claim: Option<ClickClaim>, start: bool) {
        if let Some(ClickClaim::Attack(target)) = claim
            && start
        {
            world.write_message(AttackRequest { target });
        }
    }

    fn cursor(&self, world: &mut World, _claim: ClickClaim) -> Option<Cursor> {
        Some(Cursor::fixed(world, CursorShape::Attack))
    }

    fn mark(&self, world: &mut World, claim: ClickClaim) -> Option<GroundDecal> {
        let ClickClaim::Attack(target) = claim else {
            return None;
        };
        marks::under(world, target, marks::HOSTILE)
    }
}
