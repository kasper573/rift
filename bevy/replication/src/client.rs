use bevy_app::{App, Plugin, PostUpdate, PreUpdate};
use bevy_ecs::entity::{EntityHashMap, EntityMapper};
use bevy_ecs::prelude::*;
use bevy_platform::collections::HashMap;

use crate::protocol::Protocol;
use crate::wire::{ClientFrame, EntityChanges, Part, ServerFrame, WireEntity, apply_patch};

/// Shows a client what its server replicates to it, and sends the server its messages.
pub struct ClientReplicationPlugin;

impl Plugin for ClientReplicationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Protocol>()
            .init_resource::<ServerEntities>()
            .init_resource::<ReplicatedDespawn>()
            .add_systems(PreUpdate, receive.in_set(ClientSystems::Receive))
            .add_systems(PostUpdate, send.in_set(ClientSystems::Send));
    }
}

#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ClientSystems {
    /// Applies what the server sent, and despawns everything it replicated once the
    /// `ServerLink` is gone.
    Receive,
    Send,
}

/// The client's connection to its server while it lasts, carrying the bytes of its stream both
/// ways: the app's transport hands it what arrives and sends what it has to send, reliably and in
/// order. Removing it despawns everything the server replicated.
#[derive(Resource, Default)]
pub struct ServerLink {
    /// The epoch of the server's link this client last heard from.
    epoch: u32,
    received: Vec<Vec<u8>>,
    outgoing: Vec<Vec<u8>>,
}

impl ServerLink {
    pub fn receive(&mut self, bytes: Vec<u8>) {
        self.received.push(bytes);
    }

    pub fn take_outgoing(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.outgoing)
    }
}

/// An entity the server replicates to this client.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Remote;

/// Which of this client's entities stands for which of the server's.
#[derive(Resource, Default)]
pub struct ServerEntities {
    to_client: HashMap<u64, Entity>,
    to_server: EntityHashMap<Entity>,
    /// The bytes of each kind this client holds of each of the server's entities, as the server
    /// last sent them, which its patches go over.
    held: EntityHashMap<Vec<(u16, Vec<u8>)>>,
}

impl ServerEntities {
    pub fn client_of(&self, server: Entity) -> Option<Entity> {
        self.to_client.get(&server.to_bits()).copied()
    }

    pub fn server_of(&self, client: Entity) -> Option<Entity> {
        self.to_server.get(&client).copied()
    }

    /// This client's entities that stand for one of the server's.
    pub fn iter(&self) -> impl Iterator<Item = Entity> + '_ {
        self.to_server.keys().copied()
    }

    fn insert(&mut self, server: Entity, client: Entity) {
        self.to_client.insert(server.to_bits(), client);
        self.to_server.insert(client, server);
    }

    fn forget(&mut self, server: Entity) -> Option<Entity> {
        self.held.remove(&server);
        let client = self.to_client.remove(&server.to_bits())?;
        self.to_server.remove(&client);
        Some(client)
    }
}

/// What a client does with an entity the server stopped replicating to it while connected: by
/// default it despawns it. A lost connection despawns everything outright, as the next server
/// names its entities afresh.
#[derive(Resource)]
pub struct ReplicatedDespawn(pub fn(EntityWorldMut));

impl Default for ReplicatedDespawn {
    fn default() -> ReplicatedDespawn {
        ReplicatedDespawn(|entity| entity.despawn())
    }
}

/// Names the server's entities as this client knows them: one it hasn't been sent yet gets an
/// entity of its own now, which the server's later sending fills in.
pub(crate) struct ToClientEntities<'a> {
    pub world: &'a mut World,
    pub entities: &'a mut ServerEntities,
}

impl EntityMapper for ToClientEntities<'_> {
    fn get_mapped(&mut self, source: Entity) -> Entity {
        entity_for(self.world, self.entities, source)
    }

    fn set_mapped(&mut self, source: Entity, target: Entity) {
        self.entities.insert(source, target);
    }
}

fn receive(world: &mut World) {
    let Some(mut link) = world.get_resource_mut::<ServerLink>() else {
        let_go_of_all(world);
        return;
    };
    let frames = std::mem::take(&mut link.received);
    world.resource_scope(|world, protocol: Mut<Protocol>| {
        world.resource_scope(|world, mut entities: Mut<ServerEntities>| {
            for bytes in &frames {
                apply(world, &protocol, &mut entities, bytes);
            }
        });
    });
}

fn apply(world: &mut World, protocol: &Protocol, entities: &mut ServerEntities, bytes: &[u8]) {
    let frame: ServerFrame =
        postcard::from_bytes(bytes).expect("the server speaks this client's protocol");
    let despawn = world.resource::<ReplicatedDespawn>().0;
    if let Some(epoch) = frame.fresh {
        world.resource_mut::<ServerLink>().epoch = epoch;
        for entity in std::mem::take(entities).iter().collect::<Vec<_>>() {
            if let Ok(entity) = world.get_entity_mut(entity) {
                despawn(entity);
            }
        }
    }
    for left in frame.left {
        if let Some(entity) = entities.forget(server_entity(left))
            && let Ok(entity) = world.get_entity_mut(entity)
        {
            despawn(entity);
        }
    }
    for changes in frame.entities {
        apply_changes(world, protocol, entities, changes)
            .expect("the server speaks this client's protocol");
    }
    for part in frame.messages {
        (protocol.server_messages[part.kind as usize].receive)(world, entities, part.bytes)
            .expect("the server speaks this client's protocol");
    }
}

fn apply_changes(
    world: &mut World,
    protocol: &Protocol,
    entities: &mut ServerEntities,
    changes: EntityChanges,
) -> Option<()> {
    let server = server_entity(changes.entity);
    let entity = entity_for(world, entities, server);
    for change in changes.changes {
        let kind = change.kind();
        let rule = protocol.components.get(kind as usize)?;
        let held = entities.held.entry(server).or_default();
        let at = held.iter().position(|(holds, _)| *holds == kind);
        let patched;
        let bytes = match at {
            Some(at) if change.is_patch() => {
                apply_patch(&mut held[at].1, change.bytes)?;
                patched = held[at].1.clone();
                &patched
            }
            Some(at) if change.bytes.is_empty() => {
                held.swap_remove(at);
                (rule.remove)(&mut world.entity_mut(entity));
                continue;
            }
            Some(at) => {
                held[at].1.clear();
                held[at].1.extend_from_slice(change.bytes);
                change.bytes
            }
            None if change.is_patch() => return None,
            None => {
                held.push((kind, change.bytes.to_vec()));
                change.bytes
            }
        };
        (rule.write)(world, entities, entity, bytes).ok()?;
    }
    Some(())
}

fn send(world: &mut World) {
    let mut messages: Vec<(u16, Vec<u8>)> = Vec::new();
    world.resource_scope(|world, protocol: Mut<Protocol>| {
        world.resource_scope(|world, entities: Mut<ServerEntities>| {
            let mut sent = Vec::new();
            for (kind, rule) in protocol.client_messages.iter().enumerate() {
                (rule.send)(world, &entities, &mut sent);
                let kind = u16::try_from(kind).expect("at most u16::MAX client message kinds");
                messages.extend(sent.drain(..).map(|bytes| (kind, bytes)));
            }
        });
    });
    let Some(mut link) = world.get_resource_mut::<ServerLink>() else {
        return;
    };
    if messages.is_empty() {
        return;
    }
    let frame = ClientFrame {
        epoch: link.epoch,
        messages: messages
            .iter()
            .map(|(kind, bytes)| Part {
                kind: *kind,
                bytes: bytes.as_slice(),
            })
            .collect(),
    };
    link.outgoing
        .push(postcard::to_allocvec(&frame).expect("a client frame serializes"));
}

/// The entity standing for the server's `server`, spawned now when there's none.
fn entity_for(world: &mut World, entities: &mut ServerEntities, server: Entity) -> Entity {
    if let Some(entity) = entities.client_of(server)
        && world.get_entity(entity).is_ok()
    {
        return entity;
    }
    let entity = world.spawn(Remote).id();
    entities.insert(server, entity);
    entity
}

fn let_go_of_all(world: &mut World) {
    let held: Vec<Entity> = world.resource::<ServerEntities>().iter().collect();
    if held.is_empty() {
        return;
    }
    *world.resource_mut::<ServerEntities>() = ServerEntities::default();
    for entity in held {
        if let Ok(entity) = world.get_entity_mut(entity) {
            entity.despawn();
        }
    }
}

fn server_entity(wire: WireEntity) -> Entity {
    wire.entity()
        .expect("the server names its entities by their index and generation")
}
