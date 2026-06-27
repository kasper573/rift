use bevy_ecs::message::Messages;
use bevy_ecs::prelude::*;
use bevy_time::Time;

use crate::core::math::Rng;
use crate::core::time::Seconds;
use crate::data;
use crate::systems::combat::Died;
use crate::systems::item::{Reservation, ReservedBy, scatter_drop};
use crate::systems::npc::{GameRng, Npc};
use crate::systems::player::{Players, Xp};

pub struct RewardDef {
    pub npc: data::npc::Id,
    pub amount: u32,
    pub grant: Grant,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Grant {
    Xp,
    Item {
        item: data::item::Id,
        chance: Option<f32>,
    },
}

struct GrantCtx<'a> {
    world: &'a mut World,
    rewardee: Option<Entity>,
    rng: &'a mut Rng,
    drops: &'a mut Vec<(data::item::Id, u32)>,
}

fn apply(reward: &RewardDef, ctx: &mut GrantCtx) {
    match reward.grant {
        Grant::Xp => {
            if let Some(entity) = ctx.rewardee
                && let Some(mut xp) = ctx.world.get_mut::<Xp>(entity)
            {
                xp.gain(reward.amount);
            }
        }
        Grant::Item { item, chance } => {
            if chance.is_none_or(|percent| ctx.rng.unit() * 100.0 < percent) {
                ctx.drops.push((item, reward.amount));
            }
        }
    }
}

pub fn grant(world: &mut World) {
    let now = Seconds(world.resource::<Time>().elapsed_secs());
    let deaths: Vec<Died> = world.resource_mut::<Messages<Died>>().drain().collect();
    for died in deaths {
        let Some(npc) = world.get::<Npc>(died.entity).map(|npc| npc.def) else {
            continue;
        };
        let reserved_by = match world.get::<Reservation>(died.entity) {
            Some(reservation) if !reservation.expired(now) => reservation.by,
            _ => ReservedBy::None,
        };
        let rewardee = match reserved_by {
            ReservedBy::Account(client) => world.resource::<Players>().0.get(&client).copied(),
            ReservedBy::None => None,
        };
        let mut rng = world.resource::<GameRng>().0;
        let mut drops: Vec<(data::item::Id, u32)> = Vec::new();
        for reward in data::reward::TABLE
            .values()
            .filter(|reward| reward.npc == npc)
        {
            apply(
                reward,
                &mut GrantCtx {
                    world,
                    rewardee,
                    rng: &mut rng,
                    drops: &mut drops,
                },
            );
        }
        world.resource_mut::<GameRng>().0 = rng;
        scatter_drop(world, died.entity, &drops, reserved_by);
    }
}
