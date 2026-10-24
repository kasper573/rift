use bevy::prelude::*;

use crate::core::math::Pos;
use crate::core::units::Meters;
use crate::systems::combat::{self, AttackRequest};
use crate::systems::input::gestures::{ClickClaim, Cursor, Gesture};
use crate::systems::interface::CursorShape;
use crate::systems::player::session;

pub struct AttackGesture;

impl Gesture for AttackGesture {
    fn claim(&self, world: &mut World, tile: Option<Pos<Meters>>) -> Option<ClickClaim> {
        if !session::is_alive(world) {
            return None;
        }
        combat::enemy_at(world, tile?).map(ClickClaim::Attack)
    }

    fn drive(&self, world: &mut World, claim: Option<ClickClaim>, start: bool) {
        if let Some(ClickClaim::Attack(target)) = claim
            && start
        {
            world.write_message(AttackRequest { target });
        }
    }

    fn cursor(&self, _world: &mut World, _claim: ClickClaim) -> Option<Cursor> {
        Some(Cursor::Shaped(CursorShape::Attack))
    }
}
