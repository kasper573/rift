use bevy_ecs::message::{Message, MessageCursor, Messages};
use bevy_ecs::prelude::*;

use crate::core::content::Content;
use crate::core::math::{Pos, Rng};
use crate::core::time;
use crate::core::units::Meters;
use crate::systems::GameSettings;
use crate::systems::combat::Died;
use crate::systems::item::{ItemId, Reservation, credited, scatter_drop};
use crate::systems::movement::position;
use crate::systems::npc::{Npc, NpcId};
use crate::systems::party;
use crate::systems::visibility::Presence;

#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub struct KillCredited {
    pub npc: NpcId,
    /// Where the npc fell.
    pub at: Pos<Meters>,
    /// Who the kill counts for: whoever made it, and each member of their party who shares in
    /// it, in a message of their own.
    pub credited: Entity,
}

pub fn grant(world: &mut World, mut deaths: Local<MessageCursor<Died>>) {
    let content = world.resource::<Content>().clone();
    let now = time::uptime(world);
    let claims_last = world.resource::<GameSettings>().claims_last;
    let deaths: Vec<Died> = deaths
        .read(world.resource::<Messages<Died>>())
        .cloned()
        .collect();
    for died in deaths {
        let Some(npc) = world.get::<Npc>(died.entity).map(|npc| npc.def) else {
            continue;
        };
        let reserved_by = world
            .get::<Reservation>(died.entity)
            .and_then(|reservation| reservation.holder(now, claims_last));
        let fell_at = position(world, died.entity);
        let sharers = credited(world, died.entity, now)
            .map(|credited| party::share::sharers(world, credited, fell_at))
            .unwrap_or_default();
        if let Some(at) = fell_at {
            for &credited in &sharers {
                world.write_message(KillCredited { npc, at, credited });
            }
        }
        let Some(loot) = npc.get(&content).loot() else {
            continue;
        };
        party::share::grant_xp(world, &sharers, loot.xp);
        let mut rng = world.resource_mut::<Rng>();
        let drops: Vec<(ItemId, u32)> = loot
            .items
            .iter()
            .filter_map(|drop| drop.rolled(&mut rng))
            .map(|stack| (stack.item, stack.count))
            .collect();
        let presence = world.get::<Presence>(died.entity).cloned();
        scatter_drop(world, died.entity, &drops, reserved_by, presence);
    }
}
