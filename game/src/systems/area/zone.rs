use std::collections::HashSet;

use bevy_ecs::prelude::*;

use super::{MapMarker, MarkerName};
use crate::core::assets::AssetService;
use crate::core::content::Content;
use crate::data;
use crate::systems::WorldArea;
use crate::systems::dialogue;
use crate::systems::interact;
use crate::systems::movement::position;
use crate::systems::npc::Npc;
use crate::systems::player::Players;
use crate::systems::reach::Tether;
use crate::systems::rule::{self, Encounter, Outcome, Requirement, Terms};
use crate::systems::stat;
use crate::systems::visibility;

pub struct Zone {
    pub at: MarkerName,
    pub with: Option<data::npc::Id>,
    pub requires: &'static [&'static dyn Requirement],
    pub then: &'static [&'static dyn Outcome],
}

#[derive(Component, Default)]
pub struct ApplyingZones(HashSet<usize>);

pub fn enter_zones(world: &mut World, npcs: &mut QueryState<(Entity, &'static Npc)>) {
    let content = world.resource::<Content>().clone();
    let area = world.resource::<WorldArea>().0;
    let map = world
        .resource::<AssetService>()
        .resolve(area.get(&content).map, super::build_area);
    let zones: Vec<(usize, MapMarker)> = area
        .get(&content)
        .zones
        .iter()
        .enumerate()
        .filter_map(|(index, zone)| map.marker(zone.at).map(|marker| (index, marker)))
        .collect();
    if zones.is_empty() {
        return;
    }
    let players: Vec<Entity> = world.resource::<Players>().0.values().copied().collect();
    for player in players {
        let Some(at) = position(world, player) else {
            continue;
        };
        let holding: HashSet<usize> = if stat::is_dead(world, player) {
            HashSet::new()
        } else {
            zones
                .iter()
                .filter(|&&(index, marker)| {
                    marker.covers(at)
                        && rule::met(world, player, area.get(&content).zones[index].requires)
                })
                .map(|&(index, _)| index)
                .collect()
        };
        let before = world
            .get::<ApplyingZones>(player)
            .map(|zones| zones.0.clone())
            .unwrap_or_default();
        let mut applying: HashSet<usize> = holding.intersection(&before).copied().collect();
        for &index in holding.difference(&before) {
            if dialogue::in_conversation(world, player) {
                continue;
            }
            let zone = &area.get(&content).zones[index];
            let with = zone.with.and_then(|npc| {
                npcs.iter(world)
                    .find(|&(entity, found)| {
                        found.def == npc && visibility::present(world, entity, player)
                    })
                    .map(|(entity, _)| entity)
            });
            let interaction = with.and_then(|with| interact::interaction_of(world, with));
            let encounter = Encounter {
                with,
                tether: with.zip(interaction).and_then(|(with, interaction)| {
                    Tether::around(world, player, interaction.reach, Some(with))
                }),
            };
            let terms = Terms {
                requires: zone.requires,
                costs: &[],
                outcomes: zone.then,
            };
            terms.settle(world, player, encounter).ok();
            applying.insert(index);
        }
        if before != applying {
            world.entity_mut(player).insert(ApplyingZones(applying));
        }
    }
}
