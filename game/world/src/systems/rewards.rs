use std::sync::OnceLock;

use bevy_ecs::message::Messages;
use bevy_ecs::prelude::*;
use bevy_time::Time;
use serde::{Deserialize, Deserializer};

use crate::core::table::{self, Id};
use crate::core::time::Seconds;
use crate::systems::combat::Died;
use crate::systems::items::{ItemDef, Reservation, ReservedBy, scatter_drop};
use crate::systems::npc::{GameRng, Npc, NpcDef};
use crate::systems::player::{Players, Xp};

const FILE: &str = "reward_table.json";

#[derive(Deserialize)]
pub struct Reward {
    #[serde(deserialize_with = "Id::<NpcDef>::deserialize_named")]
    pub npc: Id<NpcDef>,
    pub amount: u32,
    #[serde(flatten)]
    pub kind: RewardKind,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RewardKind {
    Xp,
    /// An absent `chance` is a guaranteed drop.
    Item {
        #[serde(deserialize_with = "Id::<ItemDef>::deserialize_named")]
        item: Id<ItemDef>,
        chance: Option<Chance>,
    },
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

/// Loot goes to the player who reserved the kill — usually the killer, but the reservation (set on
/// first attack, see [`crate::systems::items::reserve`]) makes it the player who engaged it, not
/// whoever lands the last hit. XP is granted directly; items scatter onto the map for pickup.
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
            match reward.kind {
                RewardKind::Xp => {
                    if let Some(entity) = rewardee
                        && let Some(mut xp) = world.get_mut::<Xp>(entity)
                    {
                        xp.gain(reward.amount);
                    }
                }
                RewardKind::Item { item, chance } => {
                    if chance.is_none_or(|percent| rng.unit() * 100.0 < percent.0) {
                        drops.push((item, reward.amount));
                    }
                }
            }
        }
        world.resource_mut::<GameRng>().0 = rng;
        scatter_drop(world, died.entity, &drops, reserved_by);
    }
}
