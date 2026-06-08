use std::sync::OnceLock;

use rift::{Builder, Ctx};
use serde::{Deserialize, Deserializer};

use crate::core::math::{Rng, rng_unit};
use crate::core::protocol::{Inventory, ItemId, Owner, Xp};
use crate::core::table;
use crate::features::combat::Died;
use crate::features::npc::{Npc, NpcId};

const FILE: &str = "reward_table.json";

#[derive(Deserialize)]
pub struct Reward {
    pub npc: NpcId,
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
        item: ItemId,
        chance: Option<Chance>,
    },
}

/// A drop chance in percent (fractions allowed), in (0, 100].
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

pub fn feature(b: &mut Builder) {
    b.on::<Died>(grant);
}

pub fn all() -> &'static [Reward] {
    static REWARDS: OnceLock<Vec<Reward>> = OnceLock::new();
    REWARDS.get_or_init(|| table::load(FILE))
}

pub fn rewards_for(npc: NpcId) -> impl Iterator<Item = &'static Reward> {
    all().iter().filter(move |reward| reward.npc == npc)
}

fn grant(ctx: &mut Ctx, died: &Died) {
    let Some(def) = ctx.server.world.get::<Npc>(died.entity).map(|npc| npc.def) else {
        return;
    };
    if !ctx.server.world.has::<Owner>(died.killer) {
        return;
    }
    let mut rng = ctx.res.get::<Rng>().map_or(1, |r| r.0);
    let world = &mut ctx.server.world;
    for reward in rewards_for(def) {
        match reward.kind {
            RewardKind::Xp => {
                world.modify::<Xp>(died.killer, |xp| xp.amount += reward.amount);
            }
            RewardKind::Item { item, chance } => {
                // One roll per reward row; absent chance is guaranteed.
                let granted = chance.is_none_or(|percent| rng_unit(&mut rng) * 100.0 < percent.0);
                if granted {
                    world.modify::<Inventory>(died.killer, |inventory| {
                        inventory
                            .items
                            .extend(std::iter::repeat_n(item, reward.amount as usize));
                    });
                }
            }
        }
    }
    ctx.res.insert(Rng(rng));
}
