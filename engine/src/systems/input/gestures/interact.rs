use bevy::prelude::*;

use crate::core::render::Pointed;
use crate::core::render::world::decal::GroundDecal;
use crate::systems::input::gestures::{ClickClaim, Cursor, Gesture};
use crate::systems::input::marks;
use crate::systems::interact::{self, InteractRequest};
use crate::systems::player::session;

pub struct InteractGesture;

impl Gesture for InteractGesture {
    fn claim(&self, world: &mut World, pointed: Option<&Pointed>) -> Option<ClickClaim> {
        if !session::is_alive(world) {
            return None;
        }
        interact::interactable_at(world, pointed?).map(ClickClaim::Interact)
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

    fn mark(&self, world: &mut World, claim: ClickClaim) -> Option<GroundDecal> {
        let ClickClaim::Interact(target) = claim else {
            return None;
        };
        marks::under(world, target, marks::FRIENDLY)
    }
}
