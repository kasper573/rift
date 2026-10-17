use bevy_ecs::message::{Message, MessageCursor, Messages};
use bevy_ecs::prelude::*;
use bevy_time::Time;

use crate::core::content::Content;
use crate::core::math::Rng;
use crate::core::time::Seconds;
use crate::systems::combat::Died;
use crate::systems::item::{ItemId, Reservation, credited, scatter_drop};
use crate::systems::npc::{Npc, NpcId};
use crate::systems::visibility::Presence;

#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub struct KillCredited {
    pub npc: NpcId,
    pub victim: Entity,
    pub credited: Entity,
}

pub fn grant(world: &mut World, mut deaths: Local<MessageCursor<Died>>) {
    let content = world.resource::<Content>().clone();
    let now = Seconds(world.resource::<Time>().elapsed_secs());
    let deaths: Vec<Died> = deaths
        .read(world.resource::<Messages<Died>>())
        .cloned()
        .collect();
    world.resource_scope(|world, mut rng: Mut<Rng>| {
        for died in deaths {
            let Some(npc) = world.get::<Npc>(died.entity).map(|npc| npc.def) else {
                continue;
            };
            let reserved_by = world
                .get::<Reservation>(died.entity)
                .and_then(|reservation| reservation.holder(now));
            let rewardee = credited(world, died.entity, now);
            if let Some(credited) = rewardee {
                world.write_message(KillCredited {
                    npc,
                    victim: died.entity,
                    credited,
                });
            }
            let Some(loot) = npc.get(&content).loot() else {
                continue;
            };
            if let Some(entity) = rewardee {
                crate::systems::class::grant_xp(world, entity, loot.xp);
            }
            let drops: Vec<(ItemId, u32)> = loot
                .items
                .iter()
                .filter(|drop| drop.chance.is_none_or(|chance| chance.rolled(&mut rng)))
                .map(|drop| (drop.item, drop.amount))
                .collect();
            let presence = world.get::<Presence>(died.entity).cloned();
            scatter_drop(world, died.entity, &drops, reserved_by, presence);
        }
    });
}
