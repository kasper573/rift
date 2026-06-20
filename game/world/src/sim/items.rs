use bevy_ecs::message::Messages;
use bevy_replicon::prelude::{FromClient, SendTargets, ToClients};

use crate::items::ItemKind;
use crate::protocol::{Inventory, ItemConsumed, UseItemRequest, Vitals, is_dead};

use super::player::sender_player;
use super::visibility::seen_by;

pub fn use_item(world: &mut bevy_ecs::world::World) {
    let requests: Vec<FromClient<UseItemRequest>> = world
        .resource_mut::<Messages<FromClient<UseItemRequest>>>()
        .drain()
        .collect();
    for request in requests {
        let Some(entity) = sender_player(world, request.client_id) else {
            continue;
        };
        if is_dead(world, entity) {
            continue;
        }
        let slot = request.message.slot as usize;
        let Some(slotted) = world
            .get::<Inventory>(entity)
            .and_then(|inventory| inventory.items.get(slot).copied())
        else {
            continue;
        };
        match slotted.get().kind {
            ItemKind::Consumable { health_bonus } => {
                if let Some(mut vitals) = world.get_mut::<Vitals>(entity) {
                    vitals.heal(health_bonus);
                }
                if let Some(mut inventory) = world.get_mut::<Inventory>(entity) {
                    inventory.items.remove(slot);
                }
            }
            ItemKind::Resource | ItemKind::Equipment => continue,
        }
        // Announced per beholder: a mapped message only decodes for clients that see the actor.
        for client in seen_by(world, entity) {
            world.write_message(ToClients {
                targets: SendTargets::Single(bevy_replicon::prelude::ClientId::Client(client)),
                message: ItemConsumed {
                    item: slotted,
                    actor: entity,
                },
            });
        }
    }
}
