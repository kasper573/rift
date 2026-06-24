//! Portal crossing hands players between per-area worlds: this crate raises the intent; the server harness moves them.

use bevy_ecs::prelude::*;

use super::AreaDef;
use crate::core::math::Pos;
use crate::core::table::Id;
use crate::core::tiling::Tiles;
use crate::systems::actor::Name;
use crate::systems::combat::Vitals;
use crate::systems::items::Inventory;
use crate::systems::player::{ClientId, Owner, Xp};

/// Not replicated: it never leaves the server and the entity is gone within the tick.
#[derive(Component, Clone, Copy)]
pub struct Crossing {
    pub dest_area: Id<AreaDef>,
    pub dest: Pos<Tiles>,
}

/// Transient movement, combat and pathing are dropped and rebuilt fresh on arrival.
pub struct Traveler {
    pub client: ClientId,
    pub dest_area: Id<AreaDef>,
    pub dest: Pos<Tiles>,
    pub name: String,
    pub vitals: Vitals,
    pub inventory: Inventory,
    pub xp: Xp,
}

/// Collects everyone leaving this world and despawns their character, leaving the connection behind.
/// With its character gone the connection sees nothing, so replicon sends the client a despawn for
/// every entity it held here — clearing its replication state before [`arrive`] rebuilds it in the
/// destination world over the same connection (see `game/server/src/main.rs`).
pub fn departing(world: &mut World) -> Vec<Traveler> {
    let leaving: Vec<(Entity, Traveler)> = world
        .query::<(Entity, &Owner, &Crossing, &Name, &Vitals, &Inventory, &Xp)>()
        .iter(world)
        .map(|(entity, owner, crossing, name, vitals, inventory, xp)| {
            (
                entity,
                Traveler {
                    client: owner.client,
                    dest_area: crossing.dest_area,
                    dest: crossing.dest,
                    name: name.name.clone(),
                    vitals: vitals.clone(),
                    inventory: inventory.clone(),
                    xp: xp.clone(),
                },
            )
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
        traveler.name,
        traveler.vitals,
        traveler.inventory,
        traveler.xp,
    )
}
