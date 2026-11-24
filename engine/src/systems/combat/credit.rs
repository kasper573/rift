//! Who a kill counts for: whoever made it, and the party members who share in it.

use bevy_ecs::message::{Message, MessageCursor, Messages};
use bevy_ecs::prelude::*;

use super::Died;
use crate::core::math::Pos;
use crate::core::time;
use crate::core::units::Meters;
use crate::systems::GameSettings;
use crate::systems::item::Reservation;
use crate::systems::movement::position;
use crate::systems::npc::{Npc, NpcId};
use crate::systems::party;
use crate::systems::player::{ClientId, Players};

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

/// The characters a kill of `victim` counts for: those who share in what the one credited with
/// it did where the foe fell.
pub fn sharers(world: &World, victim: Entity) -> Vec<Entity> {
    credited(world, victim)
        .map(|credited| party::sharers(world, credited, position(world, victim)))
        .unwrap_or_default()
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
