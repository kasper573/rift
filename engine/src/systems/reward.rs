//! What the game gives a player for what they did: experience and items, the one shape npcs,
//! quests and cleared instances all state theirs in, and the one way it is granted.

use bevy_ecs::message::{MessageCursor, Messages};
use bevy_ecs::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::{Content, serial};
use crate::core::math::Rng;
use crate::systems::account::AccountId;
use crate::systems::class;
use crate::systems::combat::{self, Died};
use crate::systems::item::deliver::{self, Delivered, DeliveryReason};
use crate::systems::item::{DropRow, ItemId, ItemStack, scatter_drop};
use crate::systems::npc::Npc;
use crate::systems::party;
use crate::systems::player::ClientId;
use crate::systems::society::Settling;
use crate::systems::visibility::Presence;

/// Experience and items: given whole, with an item that has a chance given only that share of
/// the times.
#[derive(JsonSchema, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reward {
    #[serde(default, skip_serializing_if = "serial::is_zero")]
    pub xp: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<DropRow>,
}

/// What a reward came to for one player this time.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rolled {
    pub xp: u32,
    /// The items that were not left to chance or that it favored, an item to a stack.
    pub items: Vec<ItemStack>,
}

/// What `grant_account` gave: the experience, and how the items were delivered.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Granted {
    pub xp: u32,
    pub delivered: Delivered,
}

/// Where a reward goes.
#[derive(Clone, Debug, PartialEq)]
pub enum Where {
    /// On the ground where `fallen` fell, for `reserved_by` to pick up first. Its experience is
    /// shared among the players as the game's parties share a kill.
    Scattered {
        fallen: Entity,
        reserved_by: Option<ClientId>,
    },
    /// Into the bag of each player, whole to each and rolled for each alone, what has no room
    /// delivered as `reason` says.
    Bag(DeliveryReason),
}

impl Reward {
    /// What the reward comes to for one player this time.
    pub fn roll(&self, rng: &mut Rng) -> Rolled {
        let mut items: Vec<ItemStack> = Vec::new();
        for stack in self.items.iter().filter_map(|row| row.rolled(rng)) {
            match items.iter_mut().find(|held| held.item == stack.item) {
                Some(held) => held.count += stack.count,
                None => items.push(stack),
            }
        }
        Rolled { xp: self.xp, items }
    }
}

/// Grants `reward` to `players`, who stand in this world, where `to` says.
pub fn grant(world: &mut World, players: &[Entity], reward: &Reward, to: Where) {
    match to {
        Where::Scattered {
            fallen,
            reserved_by,
        } => {
            let rolled = reward.roll(&mut world.resource_mut::<Rng>());
            party::share::grant_xp(world, players, rolled.xp);
            let drops: Vec<(ItemId, u32)> = rolled
                .items
                .iter()
                .map(|stack| (stack.item, stack.count))
                .collect();
            let presence = world.get::<Presence>(fallen).cloned();
            scatter_drop(world, fallen, &drops, reserved_by, presence);
        }
        Where::Bag(reason) => {
            for &player in players {
                let rolled = reward.roll(&mut world.resource_mut::<Rng>());
                class::grant_xp(world, player, rolled.xp);
                deliver::to_character(world, player, &rolled.items, reason.clone());
            }
        }
    }
}

/// Grants `rolled` to `account`, wherever they are: the experience when they are playing, the
/// items into their bag or, what has no room or they are away, by message.
pub fn grant_account(
    settling: &mut Settling,
    content: &Content,
    account: &AccountId,
    rolled: Rolled,
    reason: DeliveryReason,
) -> Granted {
    let xp = match settling.reach(account) {
        Some((world, character)) => {
            class::grant_xp(world, character, rolled.xp);
            rolled.xp
        }
        None => 0,
    };
    let delivered = deliver::to_account(settling, content, account, &rolled.items, reason);
    Granted { xp, delivered }
}

/// Grants the reward of every npc that fell to those its kill counts for: its experience shared
/// among them and its items scattered where it fell, for whoever made the kill.
pub fn kills(world: &mut World, mut deaths: Local<MessageCursor<Died>>) {
    let content = world.resource::<Content>().clone();
    let deaths: Vec<Died> = deaths
        .read(world.resource::<Messages<Died>>())
        .cloned()
        .collect();
    for died in deaths {
        let Some(npc) = world.get::<Npc>(died.entity).map(|npc| npc.def) else {
            continue;
        };
        let Some(reward) = npc.get(&content).loot() else {
            continue;
        };
        let players = combat::credit::sharers(world, died.entity);
        let to = Where::Scattered {
            fallen: died.entity,
            reserved_by: combat::credit::claimant(world, died.entity),
        };
        grant(world, &players, reward, to);
    }
}
