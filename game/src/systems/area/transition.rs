use std::collections::BTreeSet;

use bevy_ecs::prelude::*;

use super::{Id, MarkerName};
use crate::core::assets::AssetService;
use crate::core::babble::Babbler;
use crate::core::math::Pos;
use crate::core::tiling::Tiles;
use crate::systems::actor::Name;
use crate::systems::dialogue::Heard;
use crate::systems::effect::TimedEffects;
use crate::systems::equipment::Equipment;
use crate::systems::history::{self, History, HistoryEntry, HistoryMark, HistoryTopic};
use crate::systems::item::Inventory;
use crate::systems::job::Job;
use crate::systems::memory::Memory;
use crate::systems::notification;
use crate::systems::player::{CharacterState, ClientId, Owner, Xp};
use crate::systems::quest::QuestLog;
use crate::systems::reach;
use crate::systems::rule::{Outcome, RuleContext};
use crate::systems::shop;
use crate::systems::stat;
use crate::systems::text::LineText;

#[derive(Component, Clone, Copy)]
pub struct Crossing {
    pub dest_area: Id,
    pub dest: Pos<Tiles>,
}

pub struct Travel {
    pub to: Id,
    pub at: MarkerName,
}

impl Outcome for Travel {
    fn apply(&self, ctx: &mut RuleContext) {
        let assets = ctx.world.resource::<AssetService>().clone();
        let Some(dest) = super::destination(&assets, self.to, self.at) else {
            return;
        };
        reach::forget(ctx.world, ctx.player);
        ctx.world.entity_mut(ctx.player).insert(Crossing {
            dest_area: self.to,
            dest,
        });
    }

    fn check(&self, assets: &AssetService) {
        if super::destination(assets, self.to, self.at).is_none() {
            panic!(
                "travel to {:?}: marker '{}' is missing or not walkable",
                self.to, self.at.0
            );
        }
    }
}

#[derive(Component, Clone, Debug, Default)]
pub struct Discovered(BTreeSet<Id>);

impl Discovered {
    pub fn starting_in(zone: Id) -> Discovered {
        Discovered(BTreeSet::from([zone]))
    }

    pub fn discover(&mut self, area: Id) -> bool {
        self.0.insert(area)
    }
}

pub struct Traveler {
    pub client: ClientId,
    pub dest_area: Id,
    pub dest: Pos<Tiles>,
    pub state: CharacterState,
}

pub fn departing(world: &mut World) -> Vec<Traveler> {
    let ids: Vec<Entity> = world
        .query_filtered::<Entity, With<Crossing>>()
        .iter(world)
        .collect();
    let leaving: Vec<(Entity, Traveler)> = ids
        .into_iter()
        .filter_map(|entity| {
            let crossing = *world.get::<Crossing>(entity)?;
            Some((
                entity,
                Traveler {
                    client: world.get::<Owner>(entity)?.client,
                    dest_area: crossing.dest_area,
                    dest: crossing.dest,
                    state: CharacterState {
                        name: world.get::<Name>(entity)?.name.clone(),
                        babble: world.get::<Babbler>(entity).map(|babbler| babbler.0),
                        discovered: world.get::<Discovered>(entity)?.clone(),
                        stats: stat::snapshot(world, entity),
                        inventory: world.get::<Inventory>(entity)?.clone(),
                        xp: world.get::<Xp>(entity)?.clone(),
                        equipment: world.get::<Equipment>(entity)?.clone(),
                        job: *world.get::<Job>(entity)?,
                        timed: world
                            .get::<TimedEffects>(entity)
                            .cloned()
                            .unwrap_or_default(),
                        memory: world
                            .get::<Memory>(entity)
                            .cloned()
                            .map(|mut memory| {
                                memory.leave_area();
                                memory
                            })
                            .unwrap_or_default(),
                        heard: world.get::<Heard>(entity).cloned().unwrap_or_default(),
                        ledger: shop::ledger(world, entity),
                        quests: world.get::<QuestLog>(entity).cloned().unwrap_or_default(),
                        history: world.get::<History>(entity).cloned().unwrap_or_default(),
                    },
                },
            ))
        })
        .collect();
    for (entity, traveler) in &leaving {
        world
            .resource_mut::<crate::systems::player::Players>()
            .0
            .remove(&traveler.client);
        world.despawn(*entity);
    }
    leaving.into_iter().map(|(_, traveler)| traveler).collect()
}

pub fn arrive(world: &mut World, mut traveler: Traveler) -> Entity {
    let first_visit = traveler.state.discovered.discover(traveler.dest_area);
    let entity = crate::systems::player::place(
        world,
        traveler.client,
        traveler.dest_area,
        traveler.dest,
        traveler.state,
    );
    let arrived = HistoryEntry::of(
        HistoryTopic::Notification,
        LineText::plain(traveler.dest_area.get().name),
    )
    .mark(Some(HistoryMark::Arrived));
    history::record(world, entity, arrived);
    if first_visit && let Some(intro) = traveler.dest_area.get().intro {
        let notification = intro.get().for_player(world, entity);
        notification::notify(world, entity, notification);
    }
    entity
}
