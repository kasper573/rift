use bevy_ecs::message::Messages;
use bevy_ecs::prelude::*;
use bevy_time::Time;

use crate::core::math::Rng;
use crate::core::time::Seconds;
use crate::data;
use crate::systems::combat::Died;
use crate::systems::item::{Reservation, ReservedBy, scatter_drop};
use crate::systems::npc::Npc;
use crate::systems::player::{Players, Xp};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Reward {
    Xp(u32),
    Item {
        amount: u32,
        item: data::item::Id,
        chance: Option<f32>,
    },
}

pub fn grant(world: &mut World) {
    let now = Seconds(world.resource::<Time>().elapsed_secs());
    let deaths: Vec<Died> = world.resource_mut::<Messages<Died>>().drain().collect();
    world.resource_scope(|world, mut rng: Mut<Rng>| {
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
            let mut drops: Vec<(data::item::Id, u32)> = Vec::new();
            for &reward in npc.get().rewards {
                apply(
                    reward,
                    &mut RewardCtx {
                        world,
                        rewardee,
                        rng: &mut rng,
                        drops: &mut drops,
                    },
                );
            }
            scatter_drop(world, died.entity, &drops, reserved_by);
        }
    });
}

struct RewardCtx<'a> {
    world: &'a mut World,
    rewardee: Option<Entity>,
    rng: &'a mut Rng,
    drops: &'a mut Vec<(data::item::Id, u32)>,
}

fn apply(reward: Reward, ctx: &mut RewardCtx) {
    match reward {
        Reward::Xp(amount) => {
            if let Some(entity) = ctx.rewardee
                && let Some(mut xp) = ctx.world.get_mut::<Xp>(entity)
            {
                xp.gain(amount);
            }
        }
        Reward::Item {
            amount,
            item,
            chance,
        } => {
            if chance.is_none_or(|percent| ctx.rng.rand_float() * 100.0 < percent) {
                ctx.drops.push((item, amount));
            }
        }
    }
}
