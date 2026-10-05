use std::collections::HashSet;

use bevy_ecs::prelude::*;

use crate::core::tiling::{TilePos, Tiles};
use crate::data::dialogue::Id as DialogueId;
use crate::systems::area::AreaTag;
use crate::systems::interact;
use crate::systems::movement::position;
use crate::systems::player::Players;
use crate::systems::reach::Tether;
use crate::systems::rule::{self, Encounter, Outcome, Requirement, Terms};
use crate::systems::stat;
use crate::systems::visibility;

use super::Npc;

pub struct Noticing {
    pub within: Tiles,
    pub requires: &'static [&'static dyn Requirement],
    pub then: &'static [&'static dyn Outcome],
}

#[derive(Component, Default)]
pub struct Noticed(HashSet<(Entity, usize)>);

pub fn notice(world: &mut World, npcs: &mut QueryState<(Entity, &'static Npc, &'static AreaTag)>) {
    let watchers: Vec<(Entity, &'static [Noticing], crate::systems::area::Id)> = npcs
        .iter(world)
        .filter(|(npc, def, _)| !def.def.get().notices.is_empty() && !stat::is_dead(world, *npc))
        .map(|(npc, def, tag)| (npc, def.def.get().notices, tag.area))
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
        let mut noticed = HashSet::new();
        for &(npc, notices, at_area) in &watchers {
            if Some(at_area) != area || !visibility::present(world, npc, player) {
                continue;
            }
            let Some(npc_at) = position(world, npc) else {
                continue;
            };
            for (index, noticing) in notices.iter().enumerate() {
                if at.distance(npc_at) <= noticing.within
                    && rule::met(world, player, noticing.requires)
                {
                    noticed.insert((npc, index));
                }
            }
        }
        let before = world
            .get::<Noticed>(player)
            .map(|noticed| noticed.0.clone())
            .unwrap_or_default();
        for &(npc, index) in noticed.difference(&before) {
            let Some(noticing) = world
                .get::<Npc>(npc)
                .and_then(|def| def.def.get().notices.get(index))
            else {
                continue;
            };
            let range = interact::interaction_of(world, npc)
                .map_or(noticing.within, |interaction| interaction.reach);
            let encounter = Encounter {
                with: Some(npc),
                tether: Tether::around(world, player, range, Some(npc)),
            };
            let terms = Terms {
                requires: noticing.requires,
                costs: &[],
                outcomes: noticing.then,
            };
            terms.settle(world, player, encounter).ok();
        }
        if before != noticed {
            world.entity_mut(player).insert(Noticed(noticed));
        }
    }
}

pub(super) fn starts() -> Vec<DialogueId> {
    crate::data::npc::TABLE
        .iter()
        .flat_map(|def| def.notices)
        .flat_map(|noticing| noticing.then)
        .flat_map(|outcome| outcome.leads_to())
        .collect()
}
