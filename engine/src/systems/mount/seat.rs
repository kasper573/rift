//! A rider's seat: what it lets them do, and how they leave it.

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use super::{BarredAct, RiderAct, Riding, Travels};
use crate::core::content::Content;
use crate::core::math::Pos;
use crate::core::units::Meters;
use crate::systems::GameSettings;
use crate::systems::player::{self, RequestNeeds};
use crate::systems::text::LineText;

/// What a player asks of the mount they ride.
#[derive(Message, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum MountRequest {
    /// Leaves their seat: for the spot they picked, or for the ground nearest them.
    StepOff { at: Option<Pos<Meters>> },
}

pub fn requests(world: &mut World) {
    player::player_requests::<MountRequest>(
        world,
        RequestNeeds::Nothing,
        |world, player, request| match request {
            MountRequest::StepOff { at } => super::step_off(world, player, at),
        },
    );
}

/// Lets `rider` do `act` from where they sit: at once from a seat that lets them or on foot, and
/// from one that does not as their mount has it, stepping off first where they can stand or
/// refused in the game's words.
pub fn act(world: &mut World, rider: Entity, act: RiderAct) -> Result<(), LineText> {
    if let Some(refusal) = barred(world, rider, act) {
        return Err(refusal.into());
    }
    if lets(world, rider, act) {
        return Ok(());
    }
    super::step_off(world, rider, None)
}

/// What the seat `rider` holds has against them doing `act` from it, in the game's words:
/// nothing on foot, from a seat that lets them, or on a mount that has them step off for it and
/// walks the ground they would stand on. One that flies or floats refuses in its place.
pub fn barred(world: &World, rider: Entity, act: RiderAct) -> Option<String> {
    if lets(world, rider, act) {
        return None;
    }
    let def = world
        .get::<Riding>(rider)?
        .mount
        .get(world.resource::<Content>());
    let steps_off = match def.barred {
        BarredAct::StepOff => matches!(def.travels, Travels::Riding),
        BarredAct::Refuse => false,
    };
    let words = &world.resource::<GameSettings>().words;
    (!steps_off).then(|| words.not_from_mount.replace("{mount}", &def.name))
}

/// Whether `rider` may do `act` where they are with no more ado: on foot, or from a seat that
/// lets them.
pub fn lets(world: &World, rider: Entity, act: RiderAct) -> bool {
    world.get::<Riding>(rider).is_none_or(|riding| {
        let def = riding.mount.get(world.resource::<Content>());
        def.rider.may.contains(&act)
    })
}
