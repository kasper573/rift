//! Who a kill counts for: whoever made it, and the party members who share in it.

use bevy_ecs::message::{Message, MessageCursor, Messages};
use bevy_ecs::prelude::*;

use super::Died;
use crate::core::math::Pos;
use crate::core::time;
use crate::core::units::{MeterPos, Meters};
use crate::systems::GameSettings;
use crate::systems::item::Reservation;
use crate::systems::movement::position;
use crate::systems::npc::{Npc, NpcId};
use crate::systems::party;
use crate::systems::player::{Away, ClientId, Players};
use crate::systems::stat;

#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub struct KillCredited {
    pub npc: NpcId,
    /// Where the npc fell.
    pub at: Pos<Meters>,
    /// Who the kill counts for: whoever made it, and each member of their party who shares in
    /// it, in a message of their own.
    pub credited: Entity,
}

/// The client that held the claim on `victim` when it fell: whoever made the kill.
pub fn claimant(world: &World, victim: Entity) -> Option<ClientId> {
    let lasts = world.resource::<GameSettings>().claims_last;
    world
        .get::<Reservation>(victim)?
        .holder(time::uptime(world), lasts)
}

/// The character credited with `victim`'s fall: the one that held it.
pub fn credited(world: &World, victim: Entity) -> Option<Entity> {
    let client = claimant(world, victim)?;
    world.resource::<Players>().0.get(&client).copied()
}

/// The characters a kill of `victim` counts for: the one credited with it alone, or with a party
/// every member of it who stands in this world within the game's range of where the foe fell,
/// alive and with their player at them. Whoever made the kill counts from however far they made
/// it.
pub fn sharers(world: &World, victim: Entity) -> Vec<Entity> {
    let Some(credited) = credited(world, victim) else {
        return Vec::new();
    };
    let Some(rules) = world.resource::<GameSettings>().party else {
        return vec![credited];
    };
    let at = position(world, victim);
    let players = world.resource::<Players>();
    let members: Vec<Entity> = party::companions(world, credited)
        .iter()
        .filter_map(|client| players.0.get(client).copied())
        .collect();
    if members.is_empty() {
        return vec![credited];
    }
    let counts = |member: Entity| {
        let near = member == credited
            || at
                .zip(position(world, member))
                .is_some_and(|(at, stands)| at.distance(stands) <= rules.experience.range);
        near && !stat::is_dead(world, member) && world.get::<Away>(member).is_none()
    };
    members
        .into_iter()
        .filter(|&member| counts(member))
        .collect()
}

/// Tells every system that counts kills who each npc that fell counts for.
pub fn credit(world: &mut World, mut deaths: Local<MessageCursor<Died>>) {
    let deaths: Vec<Died> = deaths
        .read(world.resource::<Messages<Died>>())
        .cloned()
        .collect();
    for died in deaths {
        let Some(npc) = world.get::<Npc>(died.entity).map(|npc| npc.def) else {
            continue;
        };
        let Some(at) = position(world, died.entity) else {
            continue;
        };
        for credited in sharers(world, died.entity) {
            world.write_message(KillCredited { npc, at, credited });
        }
    }
}
