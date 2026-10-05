use std::collections::HashSet;

use bevy_ecs::prelude::*;

use super::{AreaTag, MarkerName};
use crate::data;
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
    let players: Vec<Entity> = world.resource::<Players>().0.values().copied().collect();
    for player in players {
        let (Some(at), Some(area), Some(map)) = (
            position(world, player),
            world.get::<AreaTag>(player).map(|tag| tag.area),
            super::of(world, player),
        ) else {
            continue;
        };
        let applying: HashSet<usize> = if stat::is_dead(world, player) {
            HashSet::new()
        } else {
            area.get()
                .zones
                .iter()
                .enumerate()
                .filter(|(_, zone)| {
                    map.marker(zone.at).is_some_and(|marker| marker.covers(at))
                        && rule::met(world, player, zone.requires)
                })
                .map(|(index, _)| index)
                .collect()
        };
        let before = world
            .get::<ApplyingZones>(player)
            .map(|zones| zones.0.clone())
            .unwrap_or_default();
        for &index in applying.difference(&before) {
            let zone = &area.get().zones[index];
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
        }
        if before != applying {
            world.entity_mut(player).insert(ApplyingZones(applying));
        }
    }
}
