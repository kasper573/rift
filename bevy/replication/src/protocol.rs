use std::borrow::Cow;
use std::collections::BTreeMap;

use bevy_app::App;
use bevy_ecs::component::{ComponentId, Immutable, Mutable};
use bevy_ecs::entity::{EntityMapper, MapEntities};
use bevy_ecs::message::{Message, Messages};
use bevy_ecs::prelude::*;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
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
    fn add_client_message<M: ClientMessage>(&mut self) -> &mut Self;

    /// Clients send `M`, its entities named as the server knows them.
    fn add_mapped_client_message<M: ClientMessage + MapEntities>(&mut self) -> &mut Self;

    /// The server sends `M` as `ToClient<M>`, which its client reads as `M`.
    fn add_server_message<M: Message + Serialize + DeserializeOwned>(&mut self) -> &mut Self;

    /// The server sends `M`, its entities named as the client knows them.
    fn add_mapped_server_message<M>(&mut self) -> &mut Self
    where
        M: Message + Serialize + DeserializeOwned + MapEntities;
}

/// A message a client can send: it crosses the wire, and JSON too, so whoever drives a client
/// from outside its process can hand it any message of the protocol (see
/// [`write_client_message`]).
pub trait ClientMessage: Message + Serialize + DeserializeOwned + JsonSchema {}

impl<M: Message + Serialize + DeserializeOwned + JsonSchema> ClientMessage for M {}

/// A component that can cross the wire.
pub trait Replicable:
    Component<Mutability: Overwrite<Self>> + Serialize + DeserializeOwned
{
}

impl<C: Component<Mutability: Overwrite<C>> + Serialize + DeserializeOwned> Replicable for C {}

/// `component` as a client receives it, through the encoding the wire carries it in: none for a
/// component that does not come back, which a client would drop its connection over.
pub fn over_the_wire<C: Replicable>(component: &C) -> Option<C> {
    let bytes = postcard::to_allocvec(component).ok()?;
    postcard::from_bytes(&bytes).ok()
}

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
    fn add_client_message<M: ClientMessage>(&mut self) -> &mut Self {
        add_client_message::<M>(
            self,
            ClientMessageRule {
                name: M::schema_name(),
                schema: SchemaGenerator::subschema_for::<M>,
                read: read_client_message_as::<M>,
                send: send_client_message::<M>,
                receive: receive_client_message::<M>,
            },
        )
    }

    fn add_mapped_client_message<M: ClientMessage + MapEntities>(&mut self) -> &mut Self {
        add_client_message::<M>(
            self,
            ClientMessageRule {
                name: M::schema_name(),
                schema: SchemaGenerator::subschema_for::<M>,
                read: read_mapped_client_message_as::<M>,
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

/// Writes the client message `json` holds, as the client whose world this is would write it
/// itself, so it goes to the server with the client's next send. `json` is an object of one key,
/// the message's name, over the message as serde writes it: `{"Pick": {"what": 7}}`. An entity in
/// it is named as the server knows it, and must be one the server has shown this client.
pub fn write_client_message(
    world: &mut World,
    json: serde_json::Value,
) -> Result<(), ClientMessageRefusal> {
    read_client_message(world, json).map(|message| message.write(world))
}

/// As [`write_client_message`], stopping once the message is read: whoever has more to check of
/// what was read (a name in it that serde resolved against a registry of theirs) writes it only
/// once that holds too.
pub fn read_client_message(
    world: &World,
    json: serde_json::Value,
) -> Result<ReadClientMessage, ClientMessageRefusal> {
    let named: BTreeMap<String, serde_json::Value> =
        serde_json::from_value(json).map_err(ClientMessageRefusal::Unreadable)?;
    let mut named = named.into_iter();
    let (Some((name, message)), None) = (named.next(), named.next()) else {
        return Err(ClientMessageRefusal::NotOneMessage);
    };
    let read = world
        .get_resource::<Protocol>()
        .and_then(|protocol| {
            protocol
                .client_messages
                .iter()
                .find(|rule| rule.name == name)
        })
        .map(|rule| rule.read)
        .ok_or(ClientMessageRefusal::Unknown(name))?;
    read(world, message)
}

/// A client message read from JSON, yet to be written to its client's world.
pub struct ReadClientMessage(Box<dyn FnOnce(&mut World)>);

impl ReadClientMessage {
    /// Writes the message in `world`, the client's it was read in.
    pub fn write(self, world: &mut World) {
        (self.0)(world);
    }
}

/// The JSON Schema of what [`write_client_message`] takes of the protocol `world` holds: one of
/// its client messages, under its name. What the messages are made of goes to `generator`.
pub fn client_message_schema(world: &World, generator: &mut SchemaGenerator) -> Schema {
    let messages: Vec<serde_json::Value> = world
        .get_resource::<Protocol>()
        .into_iter()
        .flat_map(|protocol| &protocol.client_messages)
        .map(|rule| {
            let named = json_schema!({
                "type": "object",
                "properties": { rule.name.as_ref(): (rule.schema)(generator) },
                "required": [rule.name],
                "additionalProperties": false,
            });
            named.to_value()
        })
        .collect();
    json_schema!({ "oneOf": messages })
}

/// Why [`write_client_message`] wrote nothing.
#[derive(Debug)]
pub enum ClientMessageRefusal {
    /// The JSON is not one message under its name.
    NotOneMessage,
    /// The protocol has no client message of this name.
    Unknown(String),
    /// The JSON does not read as the message it names.
    Unreadable(serde_json::Error),
    /// The message names an entity the server has not shown this client.
    Unseen,
}

impl std::fmt::Display for ClientMessageRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientMessageRefusal::NotOneMessage => {
                write!(f, "not one message under its name")
            }
            ClientMessageRefusal::Unknown(name) => write!(f, "no client message named {name}"),
            ClientMessageRefusal::Unreadable(error) => write!(f, "unreadable: {error}"),
            ClientMessageRefusal::Unseen => {
                write!(f, "names an entity the server has not shown this client")
            }
        }
    }
}

impl std::error::Error for ClientMessageRefusal {}

/// Which ends of the protocol an app is, so it holds each message only as its end handles it: a
/// server as `FromClient` and `ToClient`, a client as the message itself. Every world updates
/// every message it holds on every tick, so one it never reads or writes is pure cost. An app
/// that declares a message before it is given an end holds it both ways.
struct ProtocolSide {
    serves: bool,
    plays: bool,
}

impl ProtocolSide {
    fn of(app: &App) -> ProtocolSide {
        let serves = app.world().contains_resource::<crate::server::LastSend>();
        let plays = app.world().contains_resource::<ServerEntities>();
        ProtocolSide {
            serves: serves || !plays,
            plays: plays || !serves,
        }
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
    pub name: Cow<'static, str>,
    pub schema: fn(&mut SchemaGenerator) -> Schema,
    pub read: fn(&World, serde_json::Value) -> Result<ReadClientMessage, ClientMessageRefusal>,
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
    let side = ProtocolSide::of(app);
    if side.plays {
        app.add_message::<M>();
    }
    if side.serves {
        app.add_message::<FromClient<M>>();
    }
    let mut protocol = app.world_mut().get_resource_or_init::<Protocol>();
    assert!(
        protocol
            .client_messages
            .iter()
            .all(|declared| declared.name != rule.name),
        "two client messages are named {}",
        rule.name,
    );
    protocol.client_messages.push(rule);
    app
}

fn add_server_message<M: Message>(app: &mut App, rule: ServerMessageRule) -> &mut App {
    let side = ProtocolSide::of(app);
    if side.plays {
        app.add_message::<M>();
    }
    if side.serves {
        app.add_message::<ToClient<M>>();
    }
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

fn read_client_message_as<M: ClientMessage>(
    _: &World,
    json: serde_json::Value,
) -> Result<ReadClientMessage, ClientMessageRefusal> {
    let message: M = serde_json::from_value(json).map_err(ClientMessageRefusal::Unreadable)?;
    Ok(ReadClientMessage(Box::new(move |world| {
        world.write_message(message);
    })))
}

// The message is written as the client's own systems write it, its entities the client's, so it
// is sent like any other.
fn read_mapped_client_message_as<M: ClientMessage + MapEntities>(
    world: &World,
    json: serde_json::Value,
) -> Result<ReadClientMessage, ClientMessageRefusal> {
    let mut message: M = serde_json::from_value(json).map_err(ClientMessageRefusal::Unreadable)?;
    let mut seen = SeenEntities {
        entities: world.resource::<ServerEntities>(),
        all: true,
    };
    message.map_entities(&mut seen);
    if !seen.all {
        return Err(ClientMessageRefusal::Unseen);
    }
    Ok(ReadClientMessage(Box::new(move |world| {
        world.write_message(message);
    })))
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

/// Names the server's entities as a client knows them, and whether it knew them all.
struct SeenEntities<'a> {
    entities: &'a ServerEntities,
    all: bool,
}

impl EntityMapper for SeenEntities<'_> {
    fn get_mapped(&mut self, source: Entity) -> Entity {
        let seen = self.entities.client_of(source);
        self.all &= seen.is_some();
        seen.unwrap_or(Entity::PLACEHOLDER)
    }

    fn set_mapped(&mut self, _source: Entity, _target: Entity) {}
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
