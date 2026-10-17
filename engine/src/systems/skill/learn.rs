use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use super::{Learned, Rank};
use crate::core::content::Content;
use crate::systems::class::{self, SkillPoints};
use crate::systems::drain_requests;
use crate::systems::history::HistoryTopic;
use crate::systems::notification::{self, Notification, NotificationKind};
use crate::systems::player::{self, sender_player};
use crate::systems::text::LineText;

/// The whole plan the player confirmed: each node of their class tree with the rank to reach.
#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LearnSkillsRequest {
    pub ranks: Vec<(String, Rank)>,
}

pub fn requests(world: &mut World) {
    for request in drain_requests::<LearnSkillsRequest>(world) {
        let Some(player) = sender_player(world, request.client_id) else {
            continue;
        };
        if !player::may_command(world, player) {
            continue;
        }
        match learn(world, player, &request.message.ranks) {
            Ok(learned) => announce(world, player, &learned),
            Err(reason) => notification::error(world, player, reason),
        }
    }
}

/// Learns every rank of the plan or none: each node must be in the class tree, above the rank held
/// and within the skill's ranks, with its requirements met by what is held or planned, and the
/// costs together must fit the points to spend.
pub fn learn(
    world: &mut World,
    player: Entity,
    picks: &[(String, Rank)],
) -> Result<Vec<(String, Rank)>, String> {
    let content = world.resource::<Content>().clone();
    let class = class::of(world, player).ok_or("No class")?;
    let def = class.get(&content);
    let held = world.get::<Learned>(player).cloned().unwrap_or_default();
    let mut planned: BTreeMap<String, Rank> = held.0.clone();
    let mut cost = SkillPoints(0);
    let mut learned = Vec::new();
    for (key, rank) in picks {
        let node = def
            .tree
            .node(key)
            .ok_or_else(|| format!("{key} is not in your tree"))?;
        let skill = node.skill.get(&content);
        let before = planned.get(key).copied().unwrap_or(Rank::NONE);
        if *rank <= before {
            return Err(format!("{} is already rank {}", skill.name, before.0));
        }
        if rank.0 > skill.rank_count() {
            return Err(format!(
                "{} has only {} ranks",
                skill.name,
                skill.rank_count()
            ));
        }
        cost += SkillPoints(skill.cost_to(*rank).0 - skill.cost_to(before).0);
        planned.insert(key.clone(), *rank);
        learned.push((key.clone(), *rank));
    }
    for (key, _) in picks {
        let Some(node) = def.tree.node(key) else {
            continue;
        };
        if let Some(unmet) = node
            .requires
            .iter()
            .find(|requirement| !requirement.met(world, player, def, &planned))
        {
            return Err(format!(
                "{} needs {}",
                node.skill.get(&content).name,
                unmet.describe(&content, &def.tree)
            ));
        }
    }
    let available = class::points_to_spend(world, player);
    if cost > available {
        return Err(format!(
            "Needs {} skill points, you have {}",
            cost.0, available.0
        ));
    }
    world.entity_mut(player).insert(Learned(planned));
    Ok(learned)
}

fn announce(world: &mut World, player: Entity, learned: &[(String, Rank)]) {
    let content = world.resource::<Content>().clone();
    let Some(class) = class::of(world, player) else {
        return;
    };
    let tree = &class.get(&content).tree;
    for (key, rank) in learned {
        let Some(node) = tree.node(key) else {
            continue;
        };
        let skill = node.skill.get(&content);
        let text = if rank.is_first() {
            format!("Learned {} · drag it to the hotbar", skill.name)
        } else {
            format!("{} rank {}", skill.name, rank.0)
        };
        notification::notify(
            world,
            player,
            Notification::new(
                NotificationKind::Feed {
                    topic: HistoryTopic::Notification,
                    icon: Some(skill.icon.path()),
                    tally: None,
                    failure: false,
                    sfx: None,
                },
                LineText::plain(text),
            ),
        );
    }
}
