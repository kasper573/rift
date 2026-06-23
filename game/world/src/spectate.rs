//! Spectating: the replicated [`Spectate`] anchor that follows a watched player, the request to start
//! or change who's watched, and the server systems that spawn anchors and keep them on their target.

use bevy_app::App;
use bevy_ecs::component::Component;
use bevy_ecs::message::Message;
use serde::{Deserialize, Serialize};

use crate::player::ClientId;

#[cfg(feature = "systems")]
use crate::account::identity::Identity;
#[cfg(feature = "systems")]
use crate::account::role::Role;
#[cfg(feature = "systems")]
use crate::area::{self, AreaTag};
#[cfg(feature = "systems")]
use crate::movement::Position;
#[cfg(feature = "systems")]
use crate::player::{Owner, Players};
#[cfg(feature = "systems")]
use bevy_ecs::lifecycle::Remove;
#[cfg(feature = "systems")]
use bevy_ecs::message::Messages;
#[cfg(feature = "systems")]
use bevy_ecs::observer::On;
#[cfg(feature = "systems")]
use bevy_ecs::prelude::*;
#[cfg(feature = "systems")]
use bevy_replicon::prelude::{FromClient, Replicated};
#[cfg(feature = "systems")]
use std::collections::HashMap;

pub fn register(app: &mut App) {
    use bevy_replicon::prelude::*;

    app.replicate::<Spectate>()
        .add_client_message::<SpectateRequest>(Channel::Ordered);
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Spectate {
    pub watch: Option<ClientId>,
}

#[derive(Message, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SpectateRequest {
    pub watch: Option<ClientId>,
}

#[cfg(feature = "systems")]
#[derive(Resource, Default)]
pub struct Spectators(pub HashMap<ClientId, Entity>);

#[cfg(feature = "systems")]
pub fn requests(world: &mut World) {
    let requests: Vec<FromClient<SpectateRequest>> = world
        .resource_mut::<Messages<FromClient<SpectateRequest>>>()
        .drain()
        .collect();
    for request in requests {
        let Some(client_entity) = request.client_id.entity() else {
            continue;
        };
        let Some(&client) = world.get::<ClientId>(client_entity) else {
            continue;
        };
        if !allowed(world, client_entity, client) {
            continue;
        }
        let anchor = world.resource::<Spectators>().0.get(&client).copied();
        match anchor {
            Some(anchor) => {
                if let Some(mut spectate) = world.get_mut::<Spectate>(anchor) {
                    spectate.watch = request.message.watch;
                }
            }
            None => spawn_anchor(world, client, request.message.watch),
        }
    }
}

#[cfg(feature = "systems")]
fn allowed(world: &World, client_entity: Entity, client: ClientId) -> bool {
    let playing = world.resource::<Players>().0.contains_key(&client);
    let entitled = world
        .get::<Identity>(client_entity)
        .is_some_and(|identity| identity.has_role(Role::Spectate));
    !playing && entitled
}

#[cfg(feature = "systems")]
fn spawn_anchor(world: &mut World, client: ClientId, watch: Option<ClientId>) {
    let zone = world.resource::<crate::WorldArea>().0;
    let spawn = area::areas()[zone.index()].spawn;
    let entity = world
        .spawn((
            Replicated,
            Position { pos: spawn },
            AreaTag { area: zone },
            Owner { client },
            Spectate { watch },
        ))
        .id();
    world.resource_mut::<Spectators>().0.insert(client, entity);
}

#[cfg(feature = "systems")]
pub fn follow(world: &mut World) {
    let anchors: Vec<Entity> = world.resource::<Spectators>().0.values().copied().collect();
    for anchor in anchors {
        let Some(Some(watch)) = world.get::<Spectate>(anchor).map(|s| s.watch) else {
            continue;
        };
        let target = world.resource::<Players>().0.get(&watch).copied();
        let Some(player) = target else {
            if let Some(mut spectate) = world.get_mut::<Spectate>(anchor) {
                spectate.watch = None;
            }
            continue;
        };
        let (Some(at), Some(area)) = (
            world.get::<Position>(player).cloned(),
            world.get::<AreaTag>(player).cloned(),
        ) else {
            continue;
        };
        if world.get::<Position>(anchor) != Some(&at) {
            world.entity_mut(anchor).insert(at);
        }
        if world.get::<AreaTag>(anchor) != Some(&area) {
            world.entity_mut(anchor).insert(area);
        }
    }
}

#[cfg(feature = "systems")]
pub fn client_left(
    remove: On<Remove, ClientId>,
    clients: Query<(Entity, &ClientId)>,
    mut spectators: ResMut<Spectators>,
    mut commands: Commands,
) {
    let Ok((_, id)) = clients.get(remove.entity) else {
        return;
    };
    if clients
        .iter()
        .any(|(entity, other)| entity != remove.entity && other == id)
    {
        return;
    }
    if let Some(entity) = spectators.0.remove(id) {
        commands.entity(entity).despawn();
    }
}
