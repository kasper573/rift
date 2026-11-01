use bevy_app::App;
use bevy_ecs::component::{ComponentId, Immutable, Mutable};
use bevy_ecs::entity::{EntityMapper, MapEntities};
use bevy_ecs::message::{Message, Messages};
use bevy_ecs::prelude::*;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::client::{ServerEntities, ToClientEntities};

/// The components a server and its clients replicate, declared alike on both, in the same order:
/// the order numbers each kind on the wire.
pub trait AppReplicationExt {
    /// `C` goes to every client that sees its entity, and again whenever it changes.
    fn replicate<C: Replicable>(&mut self) -> &mut Self;

    /// `C` goes to every client that sees its entity as it comes into sight; later changes don't.
    fn replicate_once<C: Replicable>(&mut self) -> &mut Self;

    /// `C` goes only to the clients the interest says own its entity.
    fn replicate_private<C: Replicable>(&mut self) -> &mut Self;

    /// `C` goes only to the clients the interest says own its entity, and only while the entity
    /// has an `M` too.
    fn replicate_private_while<C: Replicable, M: Component>(&mut self) -> &mut Self;
}

/// The messages a server and its clients exchange, declared alike on both, in the same order:
/// the order numbers each kind on the wire.
pub trait AppMessageExt {
    /// Clients send `M`, which the server reads as `FromClient<M>`.
    fn add_client_message<M: Message + Serialize + DeserializeOwned>(&mut self) -> &mut Self;

    /// Clients send `M`, its entities named as the server knows them.
    fn add_mapped_client_message<M>(&mut self) -> &mut Self
    where
        M: Message + Serialize + DeserializeOwned + MapEntities;

    /// The server sends `M` as `ToClient<M>`, which its client reads as `M`.
    fn add_server_message<M: Message + Serialize + DeserializeOwned>(&mut self) -> &mut Self;

    /// The server sends `M`, its entities named as the client knows them.
    fn add_mapped_server_message<M>(&mut self) -> &mut Self
    where
        M: Message + Serialize + DeserializeOwned + MapEntities;
}

/// A component that can cross the wire.
pub trait Replicable:
    Component<Mutability: Overwrite<Self>> + Serialize + DeserializeOwned
{
}

impl<C: Component<Mutability: Overwrite<C>> + Serialize + DeserializeOwned> Replicable for C {}

/// How a client puts a received component on its entity: a mutable one changes in place, so only
/// change detection sees an update; an immutable one is inserted anew.
pub trait Overwrite<C: Component> {
    /// Puts `component` on `entity`.
    fn overwrite(entity: &mut EntityWorldMut, component: C);
}

impl<C: Component<Mutability = Mutable>> Overwrite<C> for Mutable {
    fn overwrite(entity: &mut EntityWorldMut, component: C) {
        match entity.get_mut::<C>() {
            Some(mut held) => *held = component,
            None => {
                entity.insert(component);
            }
        }
    }
}

impl<C: Component<Mutability = Immutable>> Overwrite<C> for Immutable {
    fn overwrite(entity: &mut EntityWorldMut, component: C) {
        entity.insert(component);
    }
}

/// A message a client sent, as the server reads it.
#[derive(Clone, Debug)]
pub struct FromClient<M> {
    /// The server's entity of the client that sent it.
    pub client: Entity,
    /// The message as the client wrote it.
    pub message: M,
}

impl<M: Message> Message for FromClient<M> {}

/// A message for one client, which it reads as `M`.
#[derive(Clone, Debug)]
pub struct ToClient<M> {
    /// The server's entity of the client to tell.
    pub client: Entity,
    /// The message the client reads.
    pub message: M,
}

impl<M: Message> Message for ToClient<M> {}

impl AppReplicationExt for App {
    fn replicate<C: Replicable>(&mut self) -> &mut Self {
        add_component::<C>(self, Rule::default())
    }

    fn replicate_once<C: Replicable>(&mut self) -> &mut Self {
        add_component::<C>(
            self,
            Rule {
                once: true,
                ..Rule::default()
            },
        )
    }

    fn replicate_private<C: Replicable>(&mut self) -> &mut Self {
        add_component::<C>(
            self,
            Rule {
                private: true,
                ..Rule::default()
            },
        )
    }

    fn replicate_private_while<C: Replicable, M: Component>(&mut self) -> &mut Self {
        let requires = self.world_mut().register_component::<M>();
        add_component::<C>(
            self,
            Rule {
                private: true,
                requires: Some(requires),
                ..Rule::default()
            },
        )
    }
}

impl AppMessageExt for App {
    fn add_client_message<M: Message + Serialize + DeserializeOwned>(&mut self) -> &mut Self {
        add_client_message::<M>(
            self,
            ClientMessageRule {
                send: send_client_message::<M>,
                receive: receive_client_message::<M>,
            },
        )
    }

    fn add_mapped_client_message<M>(&mut self) -> &mut Self
    where
        M: Message + Serialize + DeserializeOwned + MapEntities,
    {
        add_client_message::<M>(
            self,
            ClientMessageRule {
                send: send_mapped_client_message::<M>,
                receive: receive_client_message::<M>,
            },
        )
    }

    fn add_server_message<M: Message + Serialize + DeserializeOwned>(&mut self) -> &mut Self {
        add_server_message::<M>(
            self,
            ServerMessageRule {
                send: send_server_message::<M>,
                receive: receive_server_message::<M>,
            },
        )
    }

    fn add_mapped_server_message<M>(&mut self) -> &mut Self
    where
        M: Message + Serialize + DeserializeOwned + MapEntities,
    {
        add_server_message::<M>(
            self,
            ServerMessageRule {
                send: send_server_message::<M>,
                receive: receive_mapped_server_message::<M>,
            },
        )
    }
}

/// Every kind a server and its clients replicate, numbered by its place.
#[derive(Resource, Default)]
pub(crate) struct Protocol {
    pub components: Vec<ComponentRule>,
    pub client_messages: Vec<ClientMessageRule>,
    pub server_messages: Vec<ServerMessageRule>,
}

/// The most component kinds a protocol holds: what each client holds of an entity is a set of
/// them, kept as the bits of one integer.
pub const MAX_COMPONENT_KINDS: usize = Kinds::BITS as usize;

pub(crate) type Kinds = u128;

pub(crate) struct ComponentRule {
    pub id: ComponentId,
    pub once: bool,
    pub private: bool,
    pub requires: Option<ComponentId>,
    pub serialize: fn(EntityRef, &mut Vec<u8>),
    pub write: fn(&mut World, &mut ServerEntities, Entity, &[u8]) -> postcard::Result<()>,
    pub remove: fn(&mut EntityWorldMut),
}

pub(crate) struct ClientMessageRule {
    pub send: fn(&mut World, &ServerEntities, &mut Vec<Vec<u8>>),
    pub receive: fn(&mut World, Entity, &[u8]) -> postcard::Result<()>,
}

pub(crate) struct ServerMessageRule {
    pub send: fn(&mut World, &mut Vec<Addressed>),
    pub receive: fn(&mut World, &mut ServerEntities, &[u8]) -> postcard::Result<()>,
}

/// A serialized server message, with the server's entity of the client it is for.
pub(crate) type Addressed = (Entity, Vec<u8>);

#[derive(Default)]
struct Rule {
    once: bool,
    private: bool,
    requires: Option<ComponentId>,
}

fn add_component<C: Replicable>(app: &mut App, rule: Rule) -> &mut App {
    let id = app.world_mut().register_component::<C>();
    let mut protocol = app.world_mut().get_resource_or_init::<Protocol>();
    assert!(
        protocol.components.len() < MAX_COMPONENT_KINDS,
        "{} is replicated past the most component kinds a protocol holds, {MAX_COMPONENT_KINDS}",
        std::any::type_name::<C>(),
    );
    protocol.components.push(ComponentRule {
        id,
        once: rule.once,
        private: rule.private,
        requires: rule.requires,
        serialize: serialize_component::<C>,
        write: write_component::<C>,
        remove: |entity| {
            entity.remove::<C>();
        },
    });
    app
}

fn add_client_message<M: Message>(app: &mut App, rule: ClientMessageRule) -> &mut App {
    app.add_message::<M>().add_message::<FromClient<M>>();
    app.world_mut()
        .get_resource_or_init::<Protocol>()
        .client_messages
        .push(rule);
    app
}

fn add_server_message<M: Message>(app: &mut App, rule: ServerMessageRule) -> &mut App {
    app.add_message::<M>().add_message::<ToClient<M>>();
    app.world_mut()
        .get_resource_or_init::<Protocol>()
        .server_messages
        .push(rule);
    app
}

fn serialize_component<C: Replicable>(entity: EntityRef, bytes: &mut Vec<u8>) {
    let component = entity
        .get::<C>()
        .expect("only components an entity has are serialized");
    *bytes = postcard::to_extend(component, std::mem::take(bytes))
        .expect("a replicated component serializes");
}

fn write_component<C: Replicable>(
    world: &mut World,
    entities: &mut ServerEntities,
    entity: Entity,
    bytes: &[u8],
) -> postcard::Result<()> {
    let mut component: C = postcard::from_bytes(bytes)?;
    C::map_entities(&mut component, &mut ToClientEntities { world, entities });
    C::Mutability::overwrite(&mut world.entity_mut(entity), component);
    Ok(())
}

fn send_client_message<M: Message + Serialize>(
    world: &mut World,
    _: &ServerEntities,
    sent: &mut Vec<Vec<u8>>,
) {
    for message in world.resource_mut::<Messages<M>>().drain() {
        sent.push(postcard::to_allocvec(&message).expect("a client message serializes"));
    }
}

fn send_mapped_client_message<M: Message + Serialize + MapEntities>(
    world: &mut World,
    entities: &ServerEntities,
    sent: &mut Vec<Vec<u8>>,
) {
    for mut message in world.resource_mut::<Messages<M>>().drain() {
        message.map_entities(&mut ToServerEntities(entities));
        sent.push(postcard::to_allocvec(&message).expect("a client message serializes"));
    }
}

fn receive_client_message<M: Message + DeserializeOwned>(
    world: &mut World,
    client: Entity,
    bytes: &[u8],
) -> postcard::Result<()> {
    let message: M = postcard::from_bytes(bytes)?;
    world.write_message(FromClient { client, message });
    Ok(())
}

fn send_server_message<M: Message + Serialize>(world: &mut World, sent: &mut Vec<Addressed>) {
    for told in world.resource_mut::<Messages<ToClient<M>>>().drain() {
        let bytes = postcard::to_allocvec(&told.message).expect("a server message serializes");
        sent.push((told.client, bytes));
    }
}

fn receive_server_message<M: Message + DeserializeOwned>(
    world: &mut World,
    _: &mut ServerEntities,
    bytes: &[u8],
) -> postcard::Result<()> {
    let message: M = postcard::from_bytes(bytes)?;
    world.write_message(message);
    Ok(())
}

fn receive_mapped_server_message<M: Message + DeserializeOwned + MapEntities>(
    world: &mut World,
    entities: &mut ServerEntities,
    bytes: &[u8],
) -> postcard::Result<()> {
    let mut message: M = postcard::from_bytes(bytes)?;
    message.map_entities(&mut ToClientEntities { world, entities });
    world.write_message(message);
    Ok(())
}

/// Names a client's entities as its server knows them; one the server never sent is no entity
/// there.
struct ToServerEntities<'a>(&'a ServerEntities);

impl EntityMapper for ToServerEntities<'_> {
    fn get_mapped(&mut self, source: Entity) -> Entity {
        self.0.server_of(source).unwrap_or(Entity::PLACEHOLDER)
    }

    fn set_mapped(&mut self, _source: Entity, _target: Entity) {}
}
