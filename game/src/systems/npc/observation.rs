use std::collections::HashSet;

use bevy_ecs::prelude::*;

use crate::core::content::Content;
use crate::core::math::Pos;
use crate::core::tiling::{TilePos, Tiles};
use crate::data::dialogue::Id as DialogueId;
use crate::systems::area::AreaTag;
use crate::systems::dialogue;
use crate::systems::interact;
use crate::systems::movement::position;
use crate::systems::npc::NpcDef;
use crate::systems::player::Players;
use crate::systems::reach::Tether;
use crate::systems::rule::{self, Encounter, Outcome, Requirement, Terms};
use crate::systems::stat;
use crate::systems::visibility;

use super::Npc;

pub struct Observation {
    pub within: Tiles,
    pub requires: &'static [&'static dyn Requirement],
    pub then: &'static [&'static dyn Outcome],
}

#[derive(Component, Default)]
pub struct Observed(HashSet<(Entity, usize)>);

pub fn observe(world: &mut World, npcs: &mut QueryState<(Entity, &'static Npc, &'static AreaTag)>) {
    let content = world.resource::<Content>().clone();
    let watchers: Vec<Watcher> = npcs
        .iter(world)
        .filter(|(npc, def, _)| {
            !def.def.get(&content).observations.is_empty() && !stat::is_dead(world, *npc)
        })
        .filter_map(|(npc, def, tag)| {
            Some(Watcher {
                npc,
                observations: def.def.get(&content).observations,
                area: tag.area,
                at: position(world, npc)?,
            })
        })
        .collect();
    if watchers.is_empty() {
        return;
    }
    let players: Vec<Entity> = world.resource::<Players>().0.values().copied().collect();
    for player in players {
        if stat::is_dead(world, player) {
            continue;
        }
        let Some(at) = position(world, player) else {
            continue;
        };
        let area = world.get::<AreaTag>(player).map(|tag| tag.area);
        let mut holding = HashSet::new();
        for watcher in &watchers {
            let distance = at.distance(watcher.at);
            if Some(watcher.area) != area
                || watcher
                    .observations
                    .iter()
                    .all(|observation| distance > observation.within)
                || !visibility::present(world, watcher.npc, player)
            {
                continue;
            }
            for (index, observation) in watcher.observations.iter().enumerate() {
                if distance <= observation.within && rule::met(world, player, observation.requires)
                {
                    holding.insert((watcher.npc, index));
                }
            }
        }
        let before = world
            .get::<Observed>(player)
            .map(|observed| observed.0.clone())
            .unwrap_or_default();
        let mut observed: HashSet<(Entity, usize)> =
            holding.intersection(&before).copied().collect();
        for &(npc, index) in holding.difference(&before) {
            if dialogue::in_conversation(world, player) {
                continue;
            }
            let Some(observation) = world
                .get::<Npc>(npc)
                .and_then(|def| def.def.get(&content).observations.get(index))
            else {
                continue;
            };
            let range = interact::interaction_of(world, npc)
                .map_or(observation.within, |interaction| interaction.reach);
            let encounter = Encounter {
                with: Some(npc),
                tether: Tether::around(world, player, range, Some(npc)),
            };
            let terms = Terms {
                requires: observation.requires,
                costs: &[],
                outcomes: observation.then,
            };
            terms.settle(world, player, encounter).ok();
            observed.insert((npc, index));
        }
        if before != observed {
            world.entity_mut(player).insert(Observed(observed));
        }
    }
}

struct Watcher {
    npc: Entity,
    observations: &'static [Observation],
    area: crate::systems::area::Id,
    at: Pos<Tiles>,
}

pub(super) fn starts(content: &Content) -> Vec<DialogueId> {
    content
        .table::<NpcDef>()
        .rows()
        .iter()
        .flat_map(|def| def.observations)
        .flat_map(|observation| observation.then)
        .flat_map(|outcome| outcome.leads_to(content))
        .collect()
}
