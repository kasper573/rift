use bevy::prelude::*;

use crate::core::math::Pos;
use crate::core::units::Meters;
use crate::systems::input::gestures::{ClickClaim, Cursor, Gesture};
use crate::systems::interact::{self, InteractRequest};
use crate::systems::player::session;

pub struct InteractGesture;

impl Gesture for InteractGesture {
    fn claim(&self, world: &mut World, tile: Option<Pos<Meters>>) -> Option<ClickClaim> {
        if !session::is_alive(world) {
            return None;
        }
        interact::interactable_at(world, tile?).map(ClickClaim::Interact)
    }

    fn drive(&self, world: &mut World, claim: Option<ClickClaim>, start: bool) {
        if let Some(ClickClaim::Interact(target)) = claim
            && start
        {
            world.write_message(InteractRequest { target });
        }
    }

    fn cursor(&self, world: &mut World, claim: ClickClaim) -> Option<Cursor> {
        let ClickClaim::Interact(target) = claim else {
            return None;
        };
        let verb = interact::interaction_of(world, target)?.verb;
        Some(Cursor::Shaped(verb.cursor()))
    }
}
