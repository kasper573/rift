//! Portal crossing hands players between per-area worlds: this crate raises the intent; the server harness moves them.

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

/// Not replicated: it never leaves the server and the entity is gone within the tick.
#[derive(Component, Clone, Copy)]
pub struct Crossing {
    pub dest_area: Id<AreaDef>,
    pub dest: Pos<Tiles>,
}

/// A player handed between per-area worlds: where they're headed plus the [`CharacterState`] they
/// carry. Transient movement, combat and pathing are dropped and rebuilt fresh on arrival.
pub struct Traveler {
    pub client: ClientId,
    pub dest_area: Id<AreaDef>,
    pub dest: Pos<Tiles>,
    pub state: CharacterState,
}

/// Collects everyone leaving this world and despawns their character, leaving the connection behind.
/// With its character gone the connection sees nothing, so replicon sends the client a despawn for
/// every entity it held here — clearing its replication state before [`arrive`] rebuilds it in the
/// destination world over the same connection (see `game/server/src/main.rs`).
pub fn departing(world: &mut World) -> Vec<Traveler> {
    let ids: Vec<Entity> = world
        .query_filtered::<Entity, With<Crossing>>()
        .iter(world)
        .collect();
    // Snapshot needs `&World`, so read each crosser's components individually rather than in a query.
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
