# bevy_replication

Server-authoritative replication for [Bevy](https://bevyengine.org/), built to scale with what each
client sees and to use every core of the server.

The server owns the world. Each client holds a copy of the part of it the client may see, and the two
exchange messages. The crate decides what each client is told and turns it into bytes; the app's
transport carries the bytes.

## The model

The server keeps one reliable, ordered stream to each client. Every send, it asks the app's `Interest`
which replicated entities each client sees, and tells each client only the difference from what it
already holds:

- An entity that comes into a client's sight arrives whole: every replicated component it has.
- An entity the client holds arrives by what changed: components that changed, were added or were
  removed. A changed component goes as a patch over the bytes the client holds, or whole when that is
  smaller, or not at all when its bytes came out the same.
- An entity that leaves the client's sight, or is despawned or stops being replicated, is despawned on
  the client.
- Messages travel the same stream, after the entities of their send, so a message may name an entity
  that the same send brings.

Every client's frame is worked out in parallel, and each changed component is serialized once, however
many clients are told of it.

## Getting started

A server and its clients declare the same protocol: the components they replicate and the messages they
exchange, in the same order on both sides, as the order numbers each kind on the wire. A function both
apps call keeps them alike.

The server marks what it replicates with `Replicated` and spawns a `ClientLink` for each connected
client. The client inserts a `ServerLink` while it is connected, and each entity the server shows it
carries `Remote`. Between updates the transport hands each link the bytes that arrived and sends what the
link has to send.

```rust
use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_replication::{
    AppMessageExt, AppReplicationExt, ClientLink, ClientReplicationPlugin, FromClient, Remote,
    Replicated, ServerLink, ServerReplicationPlugin,
};
use serde::{Deserialize, Serialize};

#[derive(Component, Serialize, Deserialize, Debug, PartialEq)]
struct Health(u32);

#[derive(Component, Serialize, Deserialize)]
struct Name(String);

#[derive(Message, Serialize, Deserialize)]
struct Shout(String);

fn protocol(app: &mut App) {
    app.replicate::<Health>()
        .replicate_once::<Name>()
        .add_client_message::<Shout>();
}

let mut server = App::new();
server.add_plugins(ServerReplicationPlugin::default());
protocol(&mut server);
let link = server.world_mut().spawn(ClientLink::default()).id();
server
    .world_mut()
    .spawn((Replicated, Health(10), Name("wolf".into())));

let mut client = App::new();
client.add_plugins(ClientReplicationPlugin);
protocol(&mut client);
client.init_resource::<ServerLink>();
client.world_mut().write_message(Shout("hello".into()));

// A transport would carry these bytes over the network; here they cross in place.
client.update();
for bytes in client.world_mut().resource_mut::<ServerLink>().take_outgoing() {
    server.world_mut().get_mut::<ClientLink>(link).unwrap().receive(bytes);
}
server.update();
for bytes in server.world_mut().get_mut::<ClientLink>(link).unwrap().take_outgoing() {
    client.world_mut().resource_mut::<ServerLink>().receive(bytes);
}
client.update();

let shouts = server.world().resource::<Messages<FromClient<Shout>>>();
assert_eq!(shouts.len(), 1, "the server heard the client");
let mut shown = client
    .world_mut()
    .query_filtered::<&Health, With<Remote>>();
assert_eq!(shown.single(client.world()).unwrap(), &Health(10));
```

Components replicate in three ways, each declared once in the protocol:

- `replicate` sends a component as the entity comes into sight, and again whenever it changes.
- `replicate_once` sends it only as the entity comes into sight; later changes stay on the server.
- `replicate_private` sends it only to the clients the interest says own the entity, and
  `replicate_private_while::<C, M>` only while the entity also has an `M`, which itself need not be
  replicated. A client that stops owning the entity, or an entity that loses its `M`, has the
  component removed on the client.

A client puts a received component on its entity in place when the component is mutable, so change
detection sees an update, and inserts it anew when it is immutable.

## Interest

`Interest` decides what each client sees and which entities it owns. The default, `Everyone`, shows
every replicated entity to every client and gives none of them to anyone. An app replaces it through
the `ReplicationInterest` resource.

Every send calls `prepare` once with the world mutable, then `seen_by` and `owns` for all clients at
once, on every core, with the world shared. `prepare` is where an interest indexes the world, so that
each client's question is a lookup rather than a scan. `seen_by` may list an entity more than once, or
one that isn't replicated: duplicates count once, and what isn't replicated is not shown.

A spatial grid is the usual interest: `prepare` files every replicated entity by its cell, and a client
sees the cells around the entity it plays.

```rust
use std::collections::HashMap;

use bevy_ecs::prelude::*;
use bevy_replication::{Interest, Replicated, ReplicationInterest};

#[derive(Component, Clone, Copy, PartialEq, Eq, Hash)]
struct Cell {
    x: i32,
    y: i32,
}

/// Beside a client's `ClientLink`: the entity it plays.
#[derive(Component)]
struct Plays(Entity);

#[derive(Default)]
struct Grid {
    filed: HashMap<Cell, Vec<Entity>>,
}

impl Interest for Grid {
    fn prepare(&mut self, world: &mut World) {
        self.filed.clear();
        let mut placed = world.query_filtered::<(Entity, &Cell), With<Replicated>>();
        for (entity, cell) in placed.iter(world) {
            self.filed.entry(*cell).or_default().push(entity);
        }
    }

    fn seen_by(&self, world: &World, client: Entity, seen: &mut Vec<Entity>) {
        let Some(&at) = world
            .get::<Plays>(client)
            .and_then(|plays| world.get::<Cell>(plays.0))
        else {
            return;
        };
        for x in at.x - 1..=at.x + 1 {
            for y in at.y - 1..=at.y + 1 {
                seen.extend(self.filed.get(&Cell { x, y }).into_iter().flatten());
            }
        }
    }

    fn owns(&self, world: &World, client: Entity, entity: Entity) -> bool {
        world.get::<Plays>(client).is_some_and(|plays| plays.0 == entity)
    }
}

let mut world = World::new();
world.insert_resource(ReplicationInterest(Box::new(Grid::default())));
```

## Messages and entity mapping

Messages are bevy messages. The client writes `M` and the server reads `FromClient<M>`, which names the
`ClientLink` entity of the client that sent it. The server writes `ToClient<M>`, addressed to a
`ClientLink` entity, and that client reads `M`. Messages of either side go with the next send, those of
one kind in the order they were written.

The two sides name an entity differently: the server by its own entity, the client by the entity that
stands for it. `ServerEntities` translates between them on the client. Components and messages that
hold entities derive `MapEntities` and are translated on the way: a replicated component always is,
and a message when it is declared with `add_mapped_client_message` or `add_mapped_server_message`. A
server entity that the client hasn't been shown yet gets an entity of its own at once, which the
server's later sends fill in. A client entity that the server never sent maps to
`Entity::PLACEHOLDER`.

```rust
use bevy_app::App;
use bevy_ecs::entity::MapEntities;
use bevy_ecs::prelude::*;
use bevy_replication::{AppMessageExt, AppReplicationExt};
use serde::{Deserialize, Serialize};

#[derive(Component, MapEntities, Serialize, Deserialize)]
struct Follows(#[entities] Entity);

#[derive(Message, MapEntities, Serialize, Deserialize)]
struct Attack(#[entities] Entity);

#[derive(Message, Serialize, Deserialize)]
struct Defeated {
    xp: u32,
}

fn protocol(app: &mut App) {
    app.replicate::<Follows>()
        .add_mapped_client_message::<Attack>()
        .add_server_message::<Defeated>();
}

protocol(&mut App::new());
```

## When the server sends

By default the server sends every update, in `ServerSystems::Send` of `PostUpdate`, and reads what
arrived in `ServerSystems::Receive` of `PreUpdate`. A server that sends less often gives
`ServerSystems::Send` a run condition; what changes in between is told as one difference.

A server that plans its own tick can run the send itself: `ServerReplicationPlugin { sending:
Sending::ByHost }` adds no send, and the host runs a `Sender` of its own in three steps. `prepare`
takes from the world what the send holds of its clients, `plan` works out every frame from the world as
it stands, in parallel and with the world shared, and `apply` hands the frames to the links. `plan` may
run beside other work that reads the world.

```rust
use bevy_app::App;
use bevy_replication::{Sender, Sending, ServerReplicationPlugin};

let mut server = App::new();
server.add_plugins(ServerReplicationPlugin {
    sending: Sending::ByHost,
});
let mut sender = Sender::default();
let world = server.world_mut();
sender.prepare(world);
sender.plan(world);
sender.apply(world);
```

## Connections

Despawning a `ClientLink` ends everything the server holds of that client. Removing the client's
`ServerLink` despawns everything the server showed it, so the next connection starts afresh.

An entity that leaves a client's sight while it stays connected goes through `ReplicatedDespawn`,
which despawns it unless the app installs another function, for instance one that fades it out first.

One connection can move between server worlds, such as one world per map. The world the client leaves
takes the `ClientLink` off its entity and spawns `ClientLink::hand_over` of it in the world it joins.
The handed-over link's first send tells the client to drop everything it holds, and whatever the client
sent before hearing of the move is dropped, since it names the old world's entities.

## Guarantees

- What a client holds is exactly what the server's interest let it see at the last send, with each
  component as it was then, except those replicated once, which stay as they arrived.
- The changes of one send arrive together, ahead of the messages of that send.
- Messages arrive once each, and those of one kind in the order they were written.
- Each client costs a send in proportion to what it sees and what changed of that, and the clients
  are worked out in parallel.

## Requirements and limits

- The transport must be reliable and ordered, and must deliver each `Vec<u8>` a link hands it as one
  piece. The crate has no unreliable channel and no retransmission of its own.
- Both sides must declare the same protocol in the same order. The crate does not check it: the app
  makes sure a client and its server are of the same build. A client panics on a frame it can't read.
  A server marks a `ClientLink` as `misbehaved` when it receives bytes that aren't of the protocol, and
  leaves the consequence to the app.
- A protocol holds at most `MAX_COMPONENT_KINDS` component kinds.
- Components and messages are serialized with `postcard` through `serde`.
- A component that serializes to no bytes is sent as it arrives and never as a change.
- Every send tells every change: there is no bandwidth budget, prioritization or rate limit per entity.
  An app limits what a client is told through its interest and through how often it sends.
- The crate replicates state as the server last sent it. Prediction, interpolation and lag compensation
  are the app's.
