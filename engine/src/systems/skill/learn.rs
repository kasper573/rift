use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};

use super::{Learned, Rank};
use crate::core::content::Content;
use crate::systems::class::{self, AccessRequirement, ClassDef, NodeKey, SkillPoints};
use crate::systems::history::HistoryTopic;
use crate::systems::mention::Mention;
use crate::systems::notification::{self, Notification};
use crate::systems::player::RequestNeeds;
use crate::systems::player::player_requests;
use crate::systems::script::{Condition, condition};
use crate::systems::text::LineText;

/// The whole plan the player confirmed: each node of their class tree with the rank to reach.
#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LearnSkillsRequest {
    pub ranks: Vec<(NodeKey, Rank)>,
}

/// A plan checked against the tree: the ranks held once it is learned and what they cost.
pub struct Planned {
    pub ranks: BTreeMap<NodeKey, Rank>,
    pub cost: SkillPoints,
}

/// Why a plan is refused, by the pick that fails it.
#[derive(Clone)]
pub enum PlanRefusal {
    NotInTree {
        pick: usize,
    },
    NotAbove {
        pick: usize,
        held: Rank,
    },
    OutOfRange {
        pick: usize,
    },
    Unmet {
        pick: usize,
        requirement: AccessRequirement,
    },
}

pub fn requests(world: &mut World) {
    player_requests::<LearnSkillsRequest>(
        world,
        RequestNeeds::Command,
        |world, player, request| {
            let learned = learn(world, player, &request.ranks)?;
            announce(world, player, &learned);
            Ok(())
        },
    );
}

/// Learns every rank of the plan or none: each pick within the tree and the skill's ranks, above
/// the rank held, its requirements met by what is held or planned, and the costs together within
/// the points to spend.
pub fn learn(
    world: &mut World,
    player: Entity,
    picks: &[(NodeKey, Rank)],
) -> Result<Vec<(NodeKey, Rank)>, LineText> {
    let content = world.resource::<Content>().clone();
    let class = class::of(world, player).ok_or("No class")?;
    let def = class.get(&content);
    let held = world.get::<Learned>(player).cloned().unwrap_or_default();
    let planned = plan(
        &content,
        def,
        &held,
        picks,
        class::level(world, player),
        |c| condition::holds(world, player, c),
    )
    .map_err(|refusal| refusal.line(&content, def, picks))?;
    let available = class::points_to_spend(world, player);
    if planned.cost > available {
        return Err(format!(
            "Needs {} skill points, you have {}",
            planned.cost.0, available.0
        )
        .into());
    }
    world.entity_mut(player).insert(Learned(planned.ranks));
    Ok(picks.to_vec())
}

/// The one rule for what a plan may hold, for the world and for a snapshot alike.
pub fn plan(
    content: &Content,
    class: &ClassDef,
    held: &Learned,
    picks: &[(NodeKey, Rank)],
    level: u32,
    condition: impl Fn(&Condition) -> bool,
) -> Result<Planned, PlanRefusal> {
    let mut ranks = held.0.clone();
    let mut cost = SkillPoints(0);
    for (pick, (key, rank)) in picks.iter().enumerate() {
        let node = class
            .tree
            .node(key)
            .ok_or(PlanRefusal::NotInTree { pick })?;
        let skill = node.skill.get(content);
        let before = ranks.get(key).copied().unwrap_or(Rank::NONE);
        if *rank <= before {
            return Err(PlanRefusal::NotAbove { pick, held: before });
        }
        if !skill.ranks().contains(rank) {
            return Err(PlanRefusal::OutOfRange { pick });
        }
        cost += skill.cost_to(*rank) - skill.cost_to(before);
        ranks.insert(key.clone(), *rank);
    }
    for (pick, (key, _)) in picks.iter().enumerate() {
        let Some(node) = class.tree.node(key) else {
            continue;
        };
        if let Some(unmet) = node
            .requires
            .iter()
            .find(|requirement| !requirement.met_by(&ranks, level, &condition))
        {
            return Err(PlanRefusal::Unmet {
                pick,
                requirement: unmet.clone(),
            });
        }
    }
    Ok(Planned { ranks, cost })
}

impl PlanRefusal {
    pub fn pick(&self) -> usize {
        match self {
            PlanRefusal::NotInTree { pick }
            | PlanRefusal::NotAbove { pick, .. }
            | PlanRefusal::OutOfRange { pick }
            | PlanRefusal::Unmet { pick, .. } => *pick,
        }
    }

    pub fn sentence(
        &self,
        content: &Content,
        class: &ClassDef,
        picks: &[(NodeKey, Rank)],
    ) -> String {
        self.line(content, class, picks).plain_words()
    }

    pub fn line(&self, content: &Content, class: &ClassDef, picks: &[(NodeKey, Rank)]) -> LineText {
        let (key, rank) = &picks[self.pick()];
        let name = class.tree.node(key).map_or_else(
            || LineText::plain(key.to_string()),
            |node| Mention::Skill(node.skill).line(content),
        );
        match self {
            PlanRefusal::NotInTree { .. } => LineText::plain(format!("{key} is not in your tree")),
            PlanRefusal::NotAbove { held, .. } => name.then(format!(" is already rank {}", held.0)),
            PlanRefusal::OutOfRange { .. } => {
                let has = class
                    .tree
                    .node(key)
                    .map_or(0, |node| node.skill.get(content).rank_count());
                name.then(format!(" has only {has} ranks, not {}", rank.0))
            }
            PlanRefusal::Unmet { requirement, .. } => name
                .then(" needs ")
                .then(requirement.line(content, &class.tree)),
        }
    }
}

fn announce(world: &mut World, player: Entity, learned: &[(NodeKey, Rank)]) {
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
        let name = Mention::Skill(node.skill).name(&content);
        let text = if rank.is_first() {
            LineText::plain("Learned ").then(name)
        } else {
            LineText::from(name).then(format!(" rank {}", rank.0))
        };
        notification::notify(
            world,
            player,
            None,
            Notification::feed(HistoryTopic::Notification, text).icon(skill.icon.path()),
        );
    }
}
