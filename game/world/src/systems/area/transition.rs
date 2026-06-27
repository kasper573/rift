use bevy_ecs::prelude::*;

use super::AreaDef;
use crate::core::math::Pos;
use crate::core::table::Id;
use crate::core::tiling::Tiles;
use crate::systems::actor::Name;
use crate::systems::effect::TimedEffects;
use crate::systems::equipment::Equipment;
use crate::systems::item::Inventory;
use crate::systems::job::Job;
use crate::systems::player::{CharacterState, ClientId, Owner, Xp};
use crate::systems::stat::StatSet;

#[derive(Component, Clone, Copy)]
pub struct Crossing {
    pub dest_area: Id<AreaDef>,
    pub dest: Pos<Tiles>,
}

pub struct Traveler {
    pub client: ClientId,
    pub dest_area: Id<AreaDef>,
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
                        stats: StatSet::snapshot(world, entity),
                        inventory: world.get::<Inventory>(entity)?.clone(),
                        xp: world.get::<Xp>(entity)?.clone(),
                        equipment: world.get::<Equipment>(entity)?.clone(),
                        job: *world.get::<Job>(entity)?,
                        timed: world
                            .get::<TimedEffects>(entity)
                            .cloned()
                            .unwrap_or_default(),
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

pub fn arrive(world: &mut World, traveler: Traveler) -> Entity {
    crate::systems::player::place(
        world,
        traveler.client,
        traveler.dest_area,
        traveler.dest,
        traveler.state,
    )
}
