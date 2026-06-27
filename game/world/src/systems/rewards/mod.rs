mod item;
mod xp;

use std::sync::OnceLock;

use bevy_ecs::message::Messages;
use bevy_ecs::prelude::*;
use bevy_time::Time;
use serde::{Deserialize, Deserializer};

use crate::core::math::Rng;
use crate::core::table::{self, Id};
use crate::core::time::Seconds;
use crate::systems::combat::Died;
use crate::systems::item::{ItemDef, Reservation, ReservedBy, scatter_drop};
use crate::systems::npc::{GameRng, Npc, NpcDef};
use crate::systems::player::Players;

const FILE: &str = "reward_table.json";

#[derive(Deserialize)]
pub struct Reward {
    #[serde(deserialize_with = "Id::<NpcDef>::deserialize_named")]
    pub npc: Id<NpcDef>,
    pub amount: u32,
    #[serde(flatten)]
    pub kind: Box<dyn Grant>,
}

pub struct GrantCtx<'a> {
    pub world: &'a mut World,
    pub amount: u32,
    pub rewardee: Option<Entity>,
    pub rng: &'a mut Rng,
    pub drops: &'a mut Vec<(Id<ItemDef>, u32)>,
}

#[typetag::deserialize(tag = "type")]
pub trait Grant: Send + Sync {
    fn grant(&self, ctx: &mut GrantCtx);
}

#[derive(Clone, Copy)]
pub struct Chance(pub f32);

impl<'de> Deserialize<'de> for Chance {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let percent = f32::deserialize(deserializer)?;
        if !(percent > 0.0 && percent <= 100.0) {
            return Err(serde::de::Error::custom(
                "a chance is a percentage in (0, 100]",
            ));
        }
        Ok(Chance(percent))
    }
}

pub fn all() -> &'static [Reward] {
    static REWARDS: OnceLock<Vec<Reward>> = OnceLock::new();
    REWARDS.get_or_init(|| table::load(FILE))
}

pub fn rewards_for(npc: Id<NpcDef>) -> impl Iterator<Item = &'static Reward> {
    all().iter().filter(move |reward| reward.npc == npc)
}

pub fn grant(world: &mut World) {
    let now = Seconds(world.resource::<Time>().elapsed_secs());
    let deaths: Vec<Died> = world.resource_mut::<Messages<Died>>().drain().collect();
    for died in deaths {
        let Some(def) = world.get::<Npc>(died.entity).map(|npc| npc.def) else {
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
        let mut drops: Vec<(Id<ItemDef>, u32)> = Vec::new();
        for reward in rewards_for(def) {
            reward.kind.grant(&mut GrantCtx {
                world,
                amount: reward.amount,
                rewardee,
                rng: &mut rng,
                drops: &mut drops,
            });
        }
        world.resource_mut::<GameRng>().0 = rng;
        scatter_drop(world, died.entity, &drops, reserved_by);
    }
}
