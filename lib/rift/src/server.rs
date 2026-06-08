use crate::codec::{EVENT, SNAPSHOT};
use crate::codec::{Tag, Wire, tag, write_varint};
use crate::world::{ClientId, Entity, Map, OwnerFn, Set, World};

/// Opaque per-connection data from the transport's authenticator; rift only carries it across
/// shards, the game reads it back via [`Server::session`].
pub type Session = std::sync::Arc<dyn std::any::Any + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Connection {
    On(ClientId),
    Off(ClientId),
    Migrate(ClientId, Entity),
}

fn event_packet<E: Wire + 'static>(event: &E) -> Vec<u8> {
    let mut packet = Vec::new();
    EVENT.encode(&mut packet);
    tag::<E>().encode(&mut packet);
    event.encode(&mut packet);
    packet
}

// One client's snapshot, as a delta against what it already holds. Component bytes come from
// the shadow (the last-sent copy, refreshed by the flush); the wire frames components by a small
// per-type id whose DESPAWN value marks a whole-entity removal.
const DESPAWN: u8 = u8::MAX;

#[allow(clippy::too_many_arguments)]
fn delta_snapshot(
    world: &World,
    tick: u32,
    client_id: ClientId,
    desired: Option<&Set<Entity>>,
    sent: &Set<Entity>,
    dirty: &Set<(Entity, Tag)>,
    shadow: &Map<(Entity, Tag), Vec<u8>>,
    gone: &[(Entity, Tag)],
    replicated: &Map<Tag, (u8, bool)>,
    owner_of: Option<OwnerFn>,
    schema: &[Tag],
    fresh: bool,
) -> Vec<u8> {
    let visible = |entity: Entity| desired.is_none_or(|set| set.contains(&entity));
    let owned = |entity: Entity| owner_of.is_some_and(|f| f(world, entity) == Some(client_id));

    let mut out = Vec::new();
    SNAPSHOT.encode(&mut out);
    tick.encode(&mut out);
    client_id.0.encode(&mut out);
    out.push(u8::from(fresh));
    if fresh {
        out.push(schema.len() as u8);
        for &tag in schema {
            tag.encode(&mut out);
        }
    }

    let mut removals: Vec<(Entity, u8)> = Vec::new();
    for &entity in sent {
        if !world.alive(entity) || !visible(entity) {
            removals.push((entity, DESPAWN));
        }
    }
    for &(entity, tag) in gone {
        if let Some(&(id, private)) = replicated.get(&tag)
            && world.alive(entity)
            && sent.contains(&entity)
            && (!private || owned(entity))
        {
            removals.push((entity, id));
        }
    }
    write_varint(removals.len(), &mut out);
    for &(entity, id) in &removals {
        entity.0.encode(&mut out);
        out.push(id);
    }

    let mut changes: Vec<(Entity, u8, Tag)> = Vec::new();
    let add_full = |entity: Entity, changes: &mut Vec<(Entity, u8, Tag)>| {
        for tag in world.components_of(entity) {
            if let Some(&(id, private)) = replicated.get(&tag)
                && (!private || owned(entity))
            {
                changes.push((entity, id, tag));
            }
        }
    };
    match desired {
        Some(set) => {
            for &entity in set {
                if world.alive(entity) && !sent.contains(&entity) {
                    add_full(entity, &mut changes);
                }
            }
        }
        None => {
            for entity in world.all_entities() {
                if !sent.contains(&entity) {
                    add_full(entity, &mut changes);
                }
            }
        }
    }
    for &(entity, tag) in dirty {
        if let Some(&(id, private)) = replicated.get(&tag)
            && visible(entity)
            && sent.contains(&entity)
            && world.contains_tag(entity, tag)
            && (!private || owned(entity))
        {
            changes.push((entity, id, tag));
        }
    }

    write_varint(changes.len(), &mut out);
    let mut scratch = Vec::new();
    for (entity, id, tag) in changes {
        entity.0.encode(&mut out);
        out.push(id);
        if let Some(bytes) = shadow.get(&(entity, tag)) {
            write_varint(bytes.len(), &mut out);
            out.extend_from_slice(bytes);
        } else {
            // Unreachable for live components; a defensive fallback.
            scratch.clear();
            world.encode_tag(entity, tag, &mut scratch);
            write_varint(scratch.len(), &mut out);
            out.extend_from_slice(&scratch);
        }
    }
    out
}

#[derive(Default)]
pub struct Server {
    pub world: World,
    clients: Vec<ClientId>,
    sessions: Map<ClientId, Session>,
    desired: Map<ClientId, Set<Entity>>, // visible set requested this tick (absent = see all)
    sent: Map<ClientId, Set<Entity>>,    // entities each client currently holds
    inbox: Map<Tag, Vec<(ClientId, Vec<u8>)>>,
    outbound: Vec<(ClientId, Vec<u8>)>,
    fresh: Set<ClientId>, // clients awaiting their first (clearing) snapshot
    connections: Vec<Connection>,
    tick: u32,

    replicated: Map<Tag, (u8, bool)>, // component tag -> (compact wire id, owner-only)
    owner_of: Option<OwnerFn>,        // resolves who owner-only components replicate to
    schema: Vec<Tag>,                 // wire id -> tag, sent to each client on its first snapshot

    previous: Map<(Entity, Tag), Vec<u8>>, // last-sent bytes per component, to skip unchanged ones
    scratch: Vec<u8>,                      // reused encode buffer for the change comparison
    dirty_buf: Vec<(Entity, Tag)>,         // reused: this tick's raw dirty list, swapped from world
    changed: Set<(Entity, Tag)>,           // reused: the deduped, actually-changed components
}

impl Server {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn client_ids(&self) -> &[ClientId] {
        &self.clients
    }

    pub fn connect(&mut self, client_id: ClientId) {
        self.connect_as(client_id, None);
    }
    pub fn connect_as(&mut self, client_id: ClientId, session: Option<Session>) {
        if let Some(session) = session {
            self.sessions.insert(client_id, session);
        }
        if !self.clients.contains(&client_id) {
            self.clients.push(client_id);
            self.connections.push(Connection::On(client_id));
            self.fresh.insert(client_id);
        }
    }
    pub fn disconnect(&mut self, client_id: ClientId) {
        self.clients.retain(|&existing| existing != client_id);
        self.sessions.remove(&client_id);
        self.desired.remove(&client_id);
        self.sent.remove(&client_id);
        self.fresh.remove(&client_id);
        self.connections.push(Connection::Off(client_id));
    }

    pub fn session<T: 'static>(&self, client_id: ClientId) -> Option<&T> {
        self.sessions.get(&client_id)?.downcast_ref()
    }

    // Registered fresh, so the migrated client gets a full snapshot.
    pub(crate) fn migrate_in(
        &mut self,
        client_id: ClientId,
        entity: Entity,
        session: Option<Session>,
    ) {
        if let Some(session) = session {
            self.sessions.insert(client_id, session);
        }
        if !self.clients.contains(&client_id) {
            self.clients.push(client_id);
        }
        self.fresh.insert(client_id);
        self.connections
            .push(Connection::Migrate(client_id, entity));
    }
    pub fn drain_connections(&mut self) -> Vec<Connection> {
        std::mem::take(&mut self.connections)
    }

    pub fn set_visibility(&mut self, client_id: ClientId, visible: Option<Set<Entity>>) {
        match visible {
            Some(set) => {
                self.desired.insert(client_id, set);
            }
            None => {
                self.desired.remove(&client_id);
            }
        }
    }

    pub fn receive(&mut self, client_id: ClientId, mut packet: &[u8]) {
        let reader = &mut packet;
        if let Some(tag) = Tag::decode(reader) {
            self.inbox
                .entry(tag)
                .or_default()
                .push((client_id, reader.to_vec()));
        }
    }

    pub fn inject<E: Wire + 'static>(&mut self, client_id: ClientId, event: &E) {
        let mut bytes = Vec::new();
        event.encode(&mut bytes);
        self.inbox
            .entry(tag::<E>())
            .or_default()
            .push((client_id, bytes));
    }

    pub fn replicate<T: Wire + 'static>(&mut self) {
        self.register(tag::<T>(), false);
    }

    /// Like [`Self::replicate`], but the component reaches only its entity's owner (resolved
    /// through [`Self::owned_by`]); other clients never receive it. Ownership is read per
    /// snapshot: a transfer applies from the component's next change, while migrated and fresh
    /// clients always rebuild the full owned view.
    pub fn replicate_to_owner<T: Wire + 'static>(&mut self) {
        self.register(tag::<T>(), true);
    }

    pub fn owned_by(&mut self, owner_of: OwnerFn) {
        self.owner_of = Some(owner_of);
    }

    fn register(&mut self, tag: Tag, private: bool) {
        if !self.replicated.contains_key(&tag) {
            debug_assert!(
                self.schema.len() < 255,
                "rift replicates up to 254 component types"
            );
            self.replicated
                .insert(tag, (self.schema.len() as u8, private));
            self.schema.push(tag);
        }
    }

    pub fn drain_events<E: Wire + 'static>(&mut self) -> Vec<(ClientId, E)> {
        self.inbox
            .remove(&tag::<E>())
            .into_iter()
            .flatten()
            .filter_map(|(client_id, bytes)| {
                E::decode(&mut bytes.as_slice()).map(|event| (client_id, event))
            })
            .collect()
    }
    pub fn broadcast<E: Wire + 'static>(&mut self, event: &E) {
        let packet = event_packet(event);
        for &client_id in &self.clients {
            self.outbound.push((client_id, packet.clone()));
        }
    }
    pub fn flush(&mut self, _delta_time: f32) -> Vec<(ClientId, Vec<u8>)> {
        self.tick += 1;
        let tick = self.tick;
        let gone = std::mem::take(&mut self.world.gone);

        for &(entity, tag) in &gone {
            if tag == Tag(u64::MAX) {
                for &rep in &self.schema {
                    self.previous.remove(&(entity, rep));
                }
            } else {
                self.previous.remove(&(entity, tag));
            }
        }

        // Only values that actually changed since last sent survive; per-tick set/reset churn
        // nets no change most ticks and is dropped here.
        std::mem::swap(&mut self.dirty_buf, &mut self.world.dirty);
        self.changed.clear();
        for i in 0..self.dirty_buf.len() {
            let (entity, tag) = self.dirty_buf[i];
            if self.changed.contains(&(entity, tag))
                || !self.replicated.contains_key(&tag)
                || !self.world.contains_tag(entity, tag)
            {
                continue;
            }
            self.scratch.clear();
            self.world.encode_tag(entity, tag, &mut self.scratch);
            match self.previous.get_mut(&(entity, tag)) {
                Some(prev) if *prev == self.scratch => continue,
                Some(prev) => {
                    prev.clear();
                    prev.extend_from_slice(&self.scratch);
                }
                None => {
                    self.previous.insert((entity, tag), self.scratch.clone());
                }
            }
            self.changed.insert((entity, tag));
        }
        self.dirty_buf.clear();
        let dirty = &self.changed;

        let mut out = std::mem::take(&mut self.outbound);
        let clients = self.clients.clone();
        for client_id in clients {
            let desired = self.desired.remove(&client_id);
            let fresh = self.fresh.remove(&client_id);
            let sent = self.sent.entry(client_id).or_default();
            let packet = delta_snapshot(
                &self.world,
                tick,
                client_id,
                desired.as_ref(),
                sent,
                dirty,
                &self.previous,
                &gone,
                &self.replicated,
                self.owner_of,
                &self.schema,
                fresh,
            );
            match desired {
                Some(set) => {
                    *sent = set;
                    sent.retain(|&entity| self.world.alive(entity));
                }
                None => {
                    sent.clear();
                    sent.extend(self.world.all_entities());
                }
            }
            out.push((client_id, packet));
        }
        out
    }
}
