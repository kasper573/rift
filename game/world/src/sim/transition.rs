//! Portal crossing hands players between per-area worlds: this crate raises the intent; the server harness moves them.

use bevy_ecs::prelude::*;

use crate::area::AreaDef;
use crate::math::{Pos, Tiles};
use crate::protocol::{ClientId, Inventory, Name, Owner, Vitals, Xp};
use crate::table::Id;

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
            .resource_mut::<super::player::Players>()
            .0
            .remove(&traveler.client);
        world.despawn(*entity);
    }
    leaving.into_iter().map(|(_, traveler)| traveler).collect()
}

pub fn arrive(world: &mut World, client_entity: Entity, traveler: Traveler) -> Entity {
    super::player::place(
        world,
        traveler.client,
        client_entity,
        traveler.dest_area,
        traveler.dest,
        traveler.name,
        traveler.vitals,
        traveler.inventory,
        traveler.xp,
    )
}
