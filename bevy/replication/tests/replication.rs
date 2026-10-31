use bevy_app::App;
use bevy_ecs::entity::MapEntities;
use bevy_ecs::message::Messages;
use bevy_ecs::prelude::*;
use bevy_platform::collections::HashMap;
use bevy_replication::{
    AppMessageExt, AppReplicationExt, ClientLink, ClientReplicationPlugin, FromClient, Interest,
    Remote, Replicated, ReplicationInterest, ServerEntities, ServerLink, ServerReplicationPlugin,
    ToClient,
};
use serde::{Deserialize, Serialize};

#[test]
fn a_client_is_shown_what_the_server_replicates_as_it_changes() {
    let mut net = Net::new();
    let player = net.connect();
    let wolf = net
        .server
        .world_mut()
        .spawn((Replicated, Health(10), Name("wolf".into())))
        .id();
    net.server.world_mut().spawn(Health(99));
    net.step();
    assert_eq!(net.shown::<Health>(player, wolf), Some(Health(10)));
    assert_eq!(net.shown::<Name>(player, wolf), Some(Name("wolf".into())));
    assert_eq!(net.remote_count(player), 1, "only what is replicated");

    net.server.world_mut().entity_mut(wolf).insert(Health(4));
    net.server
        .world_mut()
        .entity_mut(wolf)
        .insert(Name("old wolf".into()));
    net.step();
    assert_eq!(net.shown::<Health>(player, wolf), Some(Health(4)));
    assert_eq!(
        net.shown::<Name>(player, wolf),
        Some(Name("wolf".into())),
        "what is replicated once stays as it came"
    );

    let spots = [
        Spot::new(10.0, 20.0, "den"),
        Spot::new(10.5, 20.0, "den"),
        Spot::new(10.5, 20.0, "den"),
        Spot::new(10.5, 20.0, "the old den"),
        Spot::new(-3.0, 8.25, "ford"),
    ];
    for spot in spots {
        net.server.world_mut().entity_mut(wolf).insert(spot.clone());
        net.step();
        assert_eq!(net.shown::<Spot>(player, wolf), Some(spot));
    }

    net.server
        .world_mut()
        .entity_mut(wolf)
        .remove::<(Health, Spot)>();
    net.step();
    assert_eq!(net.shown::<Health>(player, wolf), None);
    assert_eq!(net.shown::<Spot>(player, wolf), None);
    assert!(net.client_entity(player, wolf).is_some());
    net.server
        .world_mut()
        .entity_mut(wolf)
        .insert(Spot::new(1.0, 2.0, "den"));
    net.step();
    assert_eq!(
        net.shown::<Spot>(player, wolf),
        Some(Spot::new(1.0, 2.0, "den"))
    );

    net.server.world_mut().despawn(wolf);
    net.step();
    assert_eq!(net.client_entity(player, wolf), None);
    assert_eq!(net.remote_count(player), 0);
}

#[test]
fn a_client_holds_only_what_the_interest_lets_it_see() {
    let mut net = Net::new();
    let near = net.connect();
    let far = net.connect();
    let wolf = net
        .server
        .world_mut()
        .spawn((Replicated, Health(10), Name("wolf".into())))
        .id();
    net.sight(near, &[wolf]);
    net.sight(far, &[]);
    net.step();
    assert_eq!(net.shown::<Health>(near, wolf), Some(Health(10)));
    assert_eq!(net.client_entity(far, wolf), None);

    net.sight(near, &[]);
    net.server.world_mut().entity_mut(wolf).insert(Health(3));
    net.step();
    assert_eq!(net.client_entity(near, wolf), None, "out of sight is gone");

    net.sight(near, &[wolf]);
    net.sight(far, &[wolf]);
    net.step();
    for client in [near, far] {
        assert_eq!(net.shown::<Health>(client, wolf), Some(Health(3)));
        assert_eq!(
            net.shown::<Name>(client, wolf),
            Some(Name("wolf".into())),
            "back in sight comes whole"
        );
    }
}

#[test]
fn private_components_go_only_to_the_owner() {
    let mut net = Net::new();
    let owner = net.connect();
    let other = net.connect();
    let hero = net
        .server
        .world_mut()
        .spawn((Replicated, Health(30), Gold(7), Target(5)))
        .id();
    net.sight(owner, &[hero]);
    net.sight(other, &[hero]);
    net.own(owner, hero);
    net.step();
    assert_eq!(net.shown::<Health>(other, hero), Some(Health(30)));
    assert_eq!(net.shown::<Gold>(other, hero), None);
    assert_eq!(net.shown::<Gold>(owner, hero), Some(Gold(7)));
    assert_eq!(
        net.shown::<Target>(owner, hero),
        None,
        "a component kept while another is there goes without it"
    );

    net.server.world_mut().entity_mut(hero).insert(Hunting);
    net.step();
    assert_eq!(net.shown::<Target>(owner, hero), Some(Target(5)));
    assert_eq!(net.shown::<Target>(other, hero), None);

    net.server.world_mut().entity_mut(hero).remove::<Hunting>();
    net.own(other, hero);
    net.step();
    assert_eq!(net.shown::<Target>(owner, hero), None);
    assert_eq!(net.shown::<Gold>(owner, hero), None, "owned no more");
    assert_eq!(net.shown::<Gold>(other, hero), Some(Gold(7)));
}

#[test]
fn entities_are_named_as_each_side_knows_them() {
    let mut net = Net::new();
    let player = net.connect();
    let (wolf, pup) = {
        let world = net.server.world_mut();
        let pup = world.spawn((Replicated, Name("pup".into()))).id();
        let wolf = world
            .spawn((Replicated, Name("wolf".into()), Follows(pup)))
            .id();
        (wolf, pup)
    };
    let link = net.links[player];
    net.server.world_mut().write_message(ToClient {
        client: link,
        message: Pointed(wolf),
    });
    net.step();
    let (client_wolf, client_pup) = (
        net.client_entity(player, wolf).expect("the wolf is shown"),
        net.client_entity(player, pup).expect("the pup is shown"),
    );
    assert_eq!(
        net.shown::<Follows>(player, wolf),
        Some(Follows(client_pup))
    );
    let pointed: Vec<Pointed> = net.clients[player]
        .world_mut()
        .resource_mut::<Messages<Pointed>>()
        .drain()
        .collect();
    assert_eq!(
        pointed,
        [Pointed(client_wolf)],
        "a message names what its send brings"
    );

    net.clients[player]
        .world_mut()
        .write_message(Pick(client_pup));
    net.step();
    let picked: Vec<(Entity, Pick)> = net
        .server
        .world_mut()
        .resource_mut::<Messages<FromClient<Pick>>>()
        .drain()
        .map(|from| (from.client, from.message))
        .collect();
    assert_eq!(picked, [(link, Pick(pup))]);
}

#[test]
fn a_lost_connection_takes_everything_it_showed() {
    let mut net = Net::new();
    let player = net.connect();
    net.server
        .world_mut()
        .spawn((Replicated, Name("wolf".into())));
    net.step();
    assert_eq!(net.remote_count(player), 1);

    net.clients[player]
        .world_mut()
        .remove_resource::<ServerLink>();
    net.step();
    assert_eq!(net.remote_count(player), 0);
}

#[test]
fn a_client_handed_to_another_world_holds_only_that_world() {
    let mut net = Net::new();
    let player = net.connect();
    let wolf = net
        .server
        .world_mut()
        .spawn((Replicated, Name("wolf".into())))
        .id();
    net.step();
    let mut next = Net::new();
    let crow = next
        .server
        .world_mut()
        .spawn((Replicated, Name("crow".into())))
        .id();
    next.server
        .world_mut()
        .spawn((Replicated, Name("bat".into())));
    let client_wolf = net.client_entity(player, wolf).expect("the wolf is shown");
    net.clients[player]
        .world_mut()
        .write_message(Pick(client_wolf));
    let link = net
        .server
        .world_mut()
        .entity_mut(net.links[player])
        .take::<ClientLink>()
        .expect("the player's link");
    let handed = next.server.world_mut().spawn(link.hand_over()).id();
    next.clients.push(net.clients.remove(player));
    next.links.push(handed);
    next.step();
    assert_eq!(next.remote_count(0), 2, "nothing of the first world stays");
    assert_eq!(next.shown::<Name>(0, crow), Some(Name("crow".into())));
    assert!(
        next.server
            .world_mut()
            .resource_mut::<Messages<FromClient<Pick>>>()
            .drain()
            .next()
            .is_none(),
        "what named the first world's entities is dropped"
    );
}

#[test]
fn a_client_speaking_outside_the_protocol_is_marked() {
    let mut net = Net::new();
    let (honest, rogue) = (net.connect(), net.connect());
    net.clients[honest]
        .world_mut()
        .write_message(Pick(Entity::PLACEHOLDER));
    let rogue_link = net.links[rogue];
    net.server
        .world_mut()
        .get_mut::<ClientLink>(rogue_link)
        .unwrap()
        .receive(vec![1, 200, 7]);
    net.step();
    let marked = |net: &Net, client: usize| {
        net.server
            .world()
            .get::<ClientLink>(net.links[client])
            .unwrap()
            .misbehaved()
    };
    assert!(!marked(&net, honest));
    assert!(marked(&net, rogue));
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Health(u32);

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Name(String);

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Spot {
    x: f32,
    y: f32,
    label: String,
}

impl Spot {
    fn new(x: f32, y: f32, label: &str) -> Spot {
        Spot {
            x,
            y,
            label: label.to_owned(),
        }
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Gold(u32);

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Target(u32);

#[derive(Component)]
struct Hunting;

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Follows(#[entities] Entity);

#[derive(Message, MapEntities, Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Pointed(#[entities] Entity);

#[derive(Message, MapEntities, Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Pick(#[entities] Entity);

fn protocol(app: &mut App) {
    app.replicate::<Health>()
        .replicate::<Spot>()
        .replicate_once::<Name>()
        .replicate::<Follows>()
        .replicate_private::<Gold>()
        .replicate_private_while::<Target, Hunting>()
        .add_mapped_server_message::<Pointed>()
        .add_mapped_client_message::<Pick>();
}

/// A server and its clients in one process, their streams carried between updates. Until a
/// test says what a client sees, it sees everything replicated.
struct Net {
    server: App,
    clients: Vec<App>,
    links: Vec<Entity>,
}

impl Net {
    fn new() -> Net {
        let mut server = App::new();
        server.add_plugins(ServerReplicationPlugin);
        protocol(&mut server);
        server.init_resource::<Sights>();
        server.insert_resource(ReplicationInterest(Box::new(ByTable::default())));
        Net {
            server,
            clients: Vec::new(),
            links: Vec::new(),
        }
    }

    fn connect(&mut self) -> usize {
        let mut client = App::new();
        client.add_plugins(ClientReplicationPlugin);
        protocol(&mut client);
        client.init_resource::<ServerLink>();
        self.clients.push(client);
        let link = self.server.world_mut().spawn(ClientLink::default()).id();
        self.links.push(link);
        self.clients.len() - 1
    }

    fn sight(&mut self, client: usize, seen: &[Entity]) {
        let link = self.links[client];
        self.server
            .world_mut()
            .resource_mut::<Sights>()
            .seen
            .insert(link, seen.to_vec());
    }

    fn own(&mut self, client: usize, entity: Entity) {
        let link = self.links[client];
        self.server
            .world_mut()
            .resource_mut::<Sights>()
            .owners
            .insert(entity, link);
    }

    /// One round: every client sends what it wrote, the server updates, and every client takes
    /// in what the server sent.
    fn step(&mut self) {
        for (client, &link) in self.clients.iter_mut().zip(&self.links) {
            client.update();
            let sent = client
                .world_mut()
                .get_resource_mut::<ServerLink>()
                .map(|mut server| server.take_outgoing())
                .unwrap_or_default();
            let mut link = self.server.world_mut().get_mut::<ClientLink>(link).unwrap();
            for bytes in sent {
                link.receive(bytes);
            }
        }
        self.server.update();
        for (client, &link) in self.clients.iter_mut().zip(&self.links) {
            let sent = self
                .server
                .world_mut()
                .get_mut::<ClientLink>(link)
                .unwrap()
                .take_outgoing();
            if let Some(mut server) = client.world_mut().get_resource_mut::<ServerLink>() {
                for bytes in sent {
                    server.receive(bytes);
                }
            }
            client.update();
        }
    }

    fn client_entity(&self, client: usize, server: Entity) -> Option<Entity> {
        self.clients[client]
            .world()
            .resource::<ServerEntities>()
            .client_of(server)
    }

    fn shown<C: Component + Clone>(&self, client: usize, server: Entity) -> Option<C> {
        let entity = self.client_entity(client, server)?;
        self.clients[client].world().get::<C>(entity).cloned()
    }

    fn remote_count(&mut self, client: usize) -> usize {
        self.clients[client]
            .world_mut()
            .query_filtered::<(), With<Remote>>()
            .iter(self.clients[client].world())
            .count()
    }
}

/// What each client sees and which client owns what, as a test lays it out; a client nobody
/// laid out sees everything replicated.
#[derive(Resource, Default, Clone)]
struct Sights {
    seen: HashMap<Entity, Vec<Entity>>,
    owners: HashMap<Entity, Entity>,
}

#[derive(Default)]
struct ByTable {
    sights: Sights,
    replicated: Vec<Entity>,
}

impl Interest for ByTable {
    fn prepare(&mut self, world: &mut World) {
        self.sights = world.resource::<Sights>().clone();
        self.replicated = world
            .query_filtered::<Entity, With<Replicated>>()
            .iter(world)
            .collect();
    }

    fn seen_by(&self, _: &World, client: Entity, seen: &mut Vec<Entity>) {
        match self.sights.seen.get(&client) {
            Some(laid_out) => seen.extend(laid_out),
            None => seen.extend(&self.replicated),
        }
    }

    fn owns(&self, _: &World, client: Entity, entity: Entity) -> bool {
        self.sights.owners.get(&entity) == Some(&client)
    }
}
