use std::collections::HashSet;

use bevy_ecs::prelude::*;

use crate::core::math::Pos;
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
    let watchers: Vec<Watcher> = npcs
        .iter(world)
        .filter(|(npc, def, _)| !def.def.get().notices.is_empty() && !stat::is_dead(world, *npc))
        .filter_map(|(npc, def, tag)| {
            Some(Watcher {
                npc,
                notices: def.def.get().notices,
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
        let mut noticed = HashSet::new();
        for watcher in &watchers {
            let distance = at.distance(watcher.at);
            if Some(watcher.area) != area
                || watcher
                    .notices
                    .iter()
                    .all(|noticing| distance > noticing.within)
                || !visibility::present(world, watcher.npc, player)
            {
                continue;
            }
            for (index, noticing) in watcher.notices.iter().enumerate() {
                if distance <= noticing.within && rule::met(world, player, noticing.requires) {
                    noticed.insert((watcher.npc, index));
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

struct Watcher {
    npc: Entity,
    notices: &'static [Noticing],
    area: crate::systems::area::Id,
    at: Pos<Tiles>,
}

pub(super) fn starts() -> Vec<DialogueId> {
    crate::data::npc::TABLE
        .iter()
        .flat_map(|def| def.notices)
        .flat_map(|noticing| noticing.then)
        .flat_map(|outcome| outcome.leads_to())
        .collect()
}
