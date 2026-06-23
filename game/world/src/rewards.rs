use std::sync::OnceLock;

use bevy_ecs::message::Messages;
use bevy_ecs::prelude::*;
use serde::{Deserialize, Deserializer};

use crate::combat::Died;
use crate::core::table::{self, Id};
use crate::items::{Inventory, ItemDef};
use crate::npc::{GameRng, Npc, NpcDef};
use crate::player::{Owner, Xp};

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

pub fn grant(world: &mut World) {
    let deaths: Vec<Died> = world.resource_mut::<Messages<Died>>().drain().collect();
    for died in deaths {
        let Some(def) = world.get::<Npc>(died.entity).map(|npc| npc.def) else {
            continue;
        };
        if world.get::<Owner>(died.killer).is_none() {
            continue;
        }
        let mut rng = world.resource::<GameRng>().0;
        for reward in rewards_for(def) {
            match reward.kind {
                RewardKind::Xp => {
                    if let Some(mut xp) = world.get_mut::<Xp>(died.killer) {
                        xp.gain(reward.amount);
                    }
                }
                RewardKind::Item { item, chance } => {
                    let granted = chance.is_none_or(|percent| rng.unit() * 100.0 < percent.0);
                    if granted && let Some(mut inventory) = world.get_mut::<Inventory>(died.killer)
                    {
                        inventory
                            .items
                            .extend(std::iter::repeat_n(item, reward.amount as usize));
                    }
                }
            }
        }
        world.resource_mut::<GameRng>().0 = rng;
    }
}
