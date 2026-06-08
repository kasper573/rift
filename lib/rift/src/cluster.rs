use crate::app::{App, Feature};
use crate::codec::Tag;
use crate::server::{Server, Session};
use crate::world::{ClientId, Entity, Map};

/// The zone a shard owns, stamped into its resources.
pub struct Zone(pub u32);

// Below this, ticking serially beats paying to spawn worker threads.
const PARALLEL_MIN_ENTITIES: usize = 800;

// Shards never touch each other's state, which is what lets the cluster tick them in parallel.
struct Shard {
    app: App,
    server: Server,
    zone: u32,
    outbound: Vec<(ClientId, Vec<u8>)>,
}

impl Shard {
    fn tick(&mut self, dt: f32) {
        self.app.tick(&mut self.server, dt);
        self.outbound = self.server.flush(dt);
    }
}

struct Move {
    from: usize,
    to: usize,
    entity: Entity,
    components: Vec<(Tag, Vec<u8>)>,
    client: Option<ClientId>,
}

/// One shard per zone. Setting an entity's zone component is the whole migration API: the
/// cluster rebuilds the entity on that zone's shard and follows its client there.
pub struct Cluster {
    shards: Vec<Shard>,
    zone_shard: Map<u32, usize>,
    client_shard: Map<ClientId, usize>,
    sessions: Map<ClientId, Session>,
    spawn_zone: u32,
}

impl Cluster {
    pub fn new(features: &[Feature], zones: &[u32], spawn_zone: u32) -> Self {
        let mut shards = Vec::with_capacity(zones.len());
        let mut zone_shard = Map::default();
        for (index, &zone) in zones.iter().enumerate() {
            let mut app = App::new(features);
            app.insert_resource(Zone(zone));
            let mut server = Server::new();
            app.start(&mut server);
            shards.push(Shard {
                app,
                server,
                zone,
                outbound: Vec::new(),
            });
            zone_shard.insert(zone, index);
        }
        Self {
            shards,
            zone_shard,
            client_shard: Map::default(),
            sessions: Map::default(),
            spawn_zone,
        }
    }

    pub fn connect(&mut self, client: ClientId) {
        self.connect_as(client, None);
    }
    pub fn connect_as(&mut self, client: ClientId, session: Option<Session>) {
        let shard = self.zone_shard[&self.spawn_zone];
        if let Some(session) = &session {
            self.sessions.insert(client, session.clone());
        }
        self.shards[shard].server.connect_as(client, session);
        self.client_shard.insert(client, shard);
    }
    pub fn disconnect(&mut self, client: ClientId) {
        self.sessions.remove(&client);
        if let Some(shard) = self.client_shard.remove(&client) {
            self.shards[shard].server.disconnect(client);
        }
    }
    pub fn receive(&mut self, client: ClientId, packet: &[u8]) {
        if let Some(&shard) = self.client_shard.get(&client) {
            self.shards[shard].server.receive(client, packet);
        }
    }

    pub fn session<T: 'static>(&self, client: ClientId) -> Option<&T> {
        self.sessions.get(&client)?.downcast_ref()
    }

    pub fn tick(&mut self, dt: f32) -> Vec<(ClientId, Vec<u8>)> {
        let total: usize = self
            .shards
            .iter()
            .map(|s| s.server.world.entity_count())
            .sum();
        if self.shards.len() <= 1 || total < PARALLEL_MIN_ENTITIES {
            for shard in &mut self.shards {
                shard.tick(dt);
            }
        } else {
            let workers = std::thread::available_parallelism()
                .map_or(1, |n| n.get())
                .min(self.shards.len());
            let chunk = self.shards.len().div_ceil(workers);
            std::thread::scope(|scope| {
                for group in self.shards.chunks_mut(chunk) {
                    scope.spawn(move || {
                        for shard in group {
                            shard.tick(dt);
                        }
                    });
                }
            });
        }
        self.migrate();
        let mut out = Vec::new();
        for shard in &mut self.shards {
            out.append(&mut shard.outbound);
        }
        out
    }

    pub fn entities(&self) -> usize {
        self.shards
            .iter()
            .map(|shard| shard.server.world.entity_count())
            .sum()
    }

    pub fn server(&self, zone: u32) -> Option<&Server> {
        self.zone_shard.get(&zone).map(|&i| &self.shards[i].server)
    }
    pub fn connect_to(&mut self, client: ClientId, zone: u32) {
        if let Some(&shard) = self.zone_shard.get(&zone) {
            self.shards[shard].server.connect(client);
            self.client_shard.insert(client, shard);
        }
    }
    pub fn server_mut(&mut self, zone: u32) -> Option<&mut Server> {
        let index = *self.zone_shard.get(&zone)?;
        Some(&mut self.shards[index].server)
    }

    fn migrate(&mut self) {
        let mut moves = Vec::new();
        for (from, shard) in self.shards.iter().enumerate() {
            for entity in shard.server.world.all_entities() {
                let Some(zone) = shard.app.zone_of(&shard.server.world, entity) else {
                    continue;
                };
                if zone == shard.zone {
                    continue;
                }
                let Some(&to) = self.zone_shard.get(&zone) else {
                    continue;
                };
                moves.push(Move {
                    from,
                    to,
                    entity,
                    components: shard.server.world.extract(entity),
                    client: shard.app.owner_of(&shard.server.world, entity),
                });
            }
        }
        for mv in moves {
            let rebuilt = self.shards[mv.to].server.world.reconstruct(&mv.components);
            self.shards[mv.from].server.world.despawn(mv.entity);
            if let Some(client) = mv.client {
                self.shards[mv.from].server.disconnect(client);
                self.shards[mv.to].server.migrate_in(
                    client,
                    rebuilt,
                    self.sessions.get(&client).cloned(),
                );
                self.client_shard.insert(client, mv.to);
            }
        }
    }
}
