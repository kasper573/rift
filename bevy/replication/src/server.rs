use std::ops::Range;

use bevy_app::{App, Plugin, PostUpdate, PreUpdate};
use bevy_ecs::change_detection::Tick;
use bevy_ecs::component::ComponentId;
use bevy_ecs::entity::EntityHashMap;
use bevy_ecs::prelude::*;
use rayon::prelude::*;

use crate::protocol::{Kinds, Protocol};
use crate::wire::{Change, ClientFrame, EntityChanges, Part, ServerFrame, write_patch};

/// Replicates what the protocol declares of every `Replicated` entity to each `ClientLink`, as
/// far as the interest lets each see, and reads what clients send.
pub struct ServerReplicationPlugin;

impl Plugin for ServerReplicationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Protocol>()
            .init_resource::<ReplicationInterest>()
            .init_resource::<LastSend>()
            .init_resource::<LastSent>()
            .add_systems(PreUpdate, receive.in_set(ServerSystems::Receive))
            .add_systems(
                PostUpdate,
                (queue_messages, send.in_set(ServerSystems::Send)).chain(),
            );
    }
}

/// Where the server reads what clients sent, and where it sends: an app that sends less often than
/// every update gives `Send` a run condition; messages written in between wait for the next send.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ServerSystems {
    Receive,
    Send,
}

/// An entity the server replicates to whichever clients see it.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Replicated;

/// A client connected to the server, carrying the bytes of its stream both ways: the app's
/// transport hands it what arrives and sends what it has to send, reliably and in order.
/// Despawning it ends what the server holds of the client.
///
/// A link's first send tells the client that whatever it holds is void, so one connection may
/// move between worlds: the world it leaves despawns its link and hands it over, and the world
/// it joins spawns the link handed over, which starts the client afresh.
#[derive(Component, Default)]
pub struct ClientLink {
    epoch: u32,
    started: bool,
    received: Vec<Vec<u8>>,
    outgoing: Vec<Vec<u8>>,
    /// Every entity the client holds, with the component kinds it holds of each.
    known: EntityHashMap<Kinds>,
    messages: Vec<(u16, Vec<u8>)>,
    misbehaved: bool,
}

impl ClientLink {
    pub fn receive(&mut self, bytes: Vec<u8>) {
        self.received.push(bytes);
    }

    pub fn take_outgoing(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.outgoing)
    }

    /// The link that carries this one's client on in another world. What the client sent this
    /// one but the next receives is dropped, as it names this world's entities.
    pub fn hand_over(self) -> ClientLink {
        ClientLink {
            epoch: self.epoch + 1,
            outgoing: self.outgoing,
            ..ClientLink::default()
        }
    }

    /// Whether the client sent anything that isn't of the protocol, which no client of this
    /// build does.
    pub fn misbehaved(&self) -> bool {
        self.misbehaved
    }
}

/// Decides what each client sees. Every send asks it once per client, for all clients at once
/// on every core.
pub trait Interest: Send + Sync + 'static {
    /// Readies what the asking needs, once before each send.
    fn prepare(&mut self, world: &mut World) {
        let _ = world;
    }

    /// Adds every replicated entity `client` sees to `seen`, in any order.
    fn seen_by(&self, world: &World, client: Entity, seen: &mut Vec<Entity>);

    /// Whether `client` is told the private components of `entity`, which it sees.
    fn owns(&self, world: &World, client: Entity, entity: Entity) -> bool;
}

/// The interest the server replicates by.
#[derive(Resource)]
pub struct ReplicationInterest(pub Box<dyn Interest>);

impl Default for ReplicationInterest {
    fn default() -> ReplicationInterest {
        ReplicationInterest(Box::new(Everyone::default()))
    }
}

/// Every client sees every replicated entity and owns none.
#[derive(Default)]
pub struct Everyone {
    replicated: Vec<Entity>,
}

impl Interest for Everyone {
    fn prepare(&mut self, world: &mut World) {
        self.replicated = world
            .query_filtered::<Entity, With<Replicated>>()
            .iter(world)
            .collect();
    }

    fn seen_by(&self, _: &World, _: Entity, seen: &mut Vec<Entity>) {
        seen.extend(&self.replicated);
    }

    fn owns(&self, _: &World, _: Entity, _: Entity) -> bool {
        false
    }
}

/// The change tick of the last send, so the next tells what changed since.
#[derive(Resource)]
struct LastSend(Tick);

impl FromWorld for LastSend {
    fn from_world(world: &mut World) -> LastSend {
        LastSend(world.change_tick())
    }
}

/// The bytes every replicated entity some client sees last went out as, kind by kind: what every
/// client holding that kind of it holds, as each was told every change since.
#[derive(Resource, Default)]
struct LastSent(EntityHashMap<Vec<(u16, Vec<u8>)>>);

fn receive(world: &mut World, links: &mut QueryState<(Entity, &mut ClientLink)>) {
    let arrived: Vec<(Entity, u32, Vec<Vec<u8>>)> = links
        .iter_mut(world)
        .filter(|(_, link)| !link.received.is_empty())
        .map(|(client, mut link)| (client, link.epoch, std::mem::take(&mut link.received)))
        .collect();
    world.resource_scope(|world, protocol: Mut<Protocol>| {
        for (client, epoch, frames) in arrived {
            let read = frames.iter().all(|bytes| {
                let Ok(frame) = postcard::from_bytes::<ClientFrame>(bytes) else {
                    return false;
                };
                if frame.epoch != epoch {
                    return true;
                }
                frame.messages.iter().all(|part| {
                    protocol
                        .client_messages
                        .get(part.kind as usize)
                        .is_some_and(|rule| (rule.receive)(world, client, part.bytes).is_ok())
                })
            });
            if !read && let Some(mut link) = world.get_mut::<ClientLink>(client) {
                link.misbehaved = true;
            }
        }
    });
}

fn queue_messages(world: &mut World) {
    let sends: Vec<_> = world
        .resource::<Protocol>()
        .server_messages
        .iter()
        .map(|rule| rule.send)
        .collect();
    let mut sent = Vec::new();
    for (kind, send) in sends.into_iter().enumerate() {
        send(world, &mut sent);
        let kind = u16::try_from(kind).expect("at most u16::MAX server message kinds");
        for (client, bytes) in sent.drain(..) {
            if let Some(mut link) = world.get_mut::<ClientLink>(client) {
                link.messages.push((kind, bytes));
            }
        }
    }
}

/// Tells every client what changed in its sight since the last send, in four passes over every
/// core: what each client sees; what of each seen entity is there and what changed; what each
/// client must be told; then each changed component serialized once, and every client's frame
/// assembled from those bytes.
fn send(world: &mut World, links: &mut QueryState<(Entity, &mut ClientLink)>) {
    let this_run = world.change_tick();
    let last_run = std::mem::replace(&mut world.resource_mut::<LastSend>().0, this_run);
    world.resource_scope(|world, mut interest: Mut<ReplicationInterest>| {
        interest.0.prepare(world);
    });
    let mut last_sent = std::mem::take(&mut world.resource_mut::<LastSent>().0);
    let mut clients: Vec<Held> = links
        .iter_mut(world)
        .map(|(client, mut link)| Held {
            client,
            fresh: (!std::mem::replace(&mut link.started, true)).then_some(link.epoch),
            known: std::mem::take(&mut link.known),
            messages: std::mem::take(&mut link.messages),
        })
        .collect();

    let shared: &World = world;
    let protocol = shared.resource::<Protocol>();
    let interest = &*shared.resource::<ReplicationInterest>().0;
    let replicated = shared
        .component_id::<Replicated>()
        .expect("Replicated is registered once anything is");
    let private = protocol
        .components
        .iter()
        .enumerate()
        .filter(|(_, rule)| rule.private)
        .fold(0, |mask, (kind, _)| mask | bit(kind));

    let seen: Vec<Vec<Entity>> = clients
        .par_iter()
        .map(|held| {
            let mut seen = Vec::new();
            interest.seen_by(shared, held.client, &mut seen);
            seen.sort_unstable();
            seen.dedup();
            seen
        })
        .collect();

    let mut subjects: Vec<Entity> = seen.iter().flatten().copied().collect();
    subjects.sort_unstable();
    subjects.dedup();
    let summaries: Vec<Option<Summary>> = subjects
        .par_iter()
        .map(|&entity| summarize(shared, protocol, replicated, entity, last_run, this_run))
        .collect();
    let summary_of = |entity: Entity| {
        subjects
            .binary_search(&entity)
            .ok()
            .and_then(|index| summaries[index].map(|summary| (index, summary)))
    };

    let plans: Vec<Plan> = clients
        .par_iter_mut()
        .zip(&seen)
        .map(|(held, seen)| {
            let mut plan = Plan::default();
            for &entity in seen {
                let Some((subject, summary)) = summary_of(entity) else {
                    continue;
                };
                let mut want = summary.present;
                if want & private != 0 && !interest.owns(shared, held.client, entity) {
                    want &= !private;
                }
                match held.known.get_mut(&entity) {
                    None => {
                        held.known.insert(entity, want);
                        plan.entities.push(Told {
                            subject,
                            arrives: true,
                            whole: want,
                            updated: 0,
                            removed: 0,
                        });
                    }
                    Some(holds) => {
                        let told = Told {
                            subject,
                            arrives: false,
                            whole: want & !*holds,
                            updated: want & *holds & summary.changed,
                            removed: *holds & !want,
                        };
                        *holds = want;
                        if told.whole | told.updated | told.removed != 0 {
                            plan.entities.push(told);
                        }
                    }
                }
            }
            held.known.retain(|&entity, _| {
                let stays = seen.binary_search(&entity).is_ok() && summary_of(entity).is_some();
                if !stays {
                    plan.left.push(entity);
                }
                stays
            });
            plan
        })
        .collect();

    let mut needed: Vec<Kinds> = vec![0; subjects.len()];
    for plan in &plans {
        for told in &plan.entities {
            needed[told.subject] |= told.whole | told.updated;
        }
    }
    let mut sent_as: Vec<Vec<(u16, Vec<u8>)>> = subjects
        .iter()
        .map(|entity| last_sent.remove(entity).unwrap_or_default())
        .collect();
    let serialized: Vec<Serialized> = subjects
        .par_iter()
        .zip(&needed)
        .zip(&mut sent_as)
        .map(|((&entity, &kinds), sent_as)| serialize(shared, protocol, entity, kinds, sent_as))
        .collect();

    let frames: Vec<Option<Vec<u8>>> = plans
        .par_iter()
        .zip(&clients)
        .map(|(plan, held)| {
            let frame = ServerFrame {
                fresh: held.fresh,
                left: plan.left.iter().map(|&entity| entity.into()).collect(),
                entities: plan
                    .entities
                    .iter()
                    .filter_map(|told| {
                        let changes: Vec<Change> = serialized[told.subject]
                            .changes(told)
                            .chain(kinds_of(told.removed).map(|kind| Change::whole(kind, &[])))
                            .collect();
                        (told.arrives || !changes.is_empty()).then(|| EntityChanges {
                            entity: subjects[told.subject].into(),
                            changes,
                        })
                    })
                    .collect(),
                messages: held
                    .messages
                    .iter()
                    .map(|(kind, bytes)| Part {
                        kind: *kind,
                        bytes: bytes.as_slice(),
                    })
                    .collect(),
            };
            (!frame.is_empty())
                .then(|| postcard::to_allocvec(&frame).expect("a server frame serializes"))
        })
        .collect();

    world.resource_mut::<LastSent>().0 = subjects
        .into_iter()
        .zip(sent_as)
        .filter(|(_, sent_as)| !sent_as.is_empty())
        .collect();
    for (held, frame) in clients.into_iter().zip(frames) {
        let Some(mut link) = world.get_mut::<ClientLink>(held.client) else {
            continue;
        };
        link.known = held.known;
        link.outgoing.extend(frame);
    }
}

/// What a send holds of a client while it works out what to tell it.
struct Held {
    client: Entity,
    fresh: Option<u32>,
    known: EntityHashMap<Kinds>,
    messages: Vec<(u16, Vec<u8>)>,
}

#[derive(Clone, Copy)]
struct Summary {
    /// The kinds the entity has, of those whose rules let them go out now.
    present: Kinds,
    /// The kinds among them that changed since the last send.
    changed: Kinds,
}

/// What to tell a client: what left its sight, and what to tell it of each entity in it.
#[derive(Default)]
struct Plan {
    left: Vec<Entity>,
    entities: Vec<Told>,
}

/// What to tell a client of one entity, by its place among the send's subjects: whether it comes
/// into the client's sight; the kinds the client doesn't hold, told whole; those it holds that
/// changed; and those it holds but may no longer.
struct Told {
    subject: usize,
    arrives: bool,
    whole: Kinds,
    updated: Kinds,
    removed: Kinds,
}

/// An entity's components that some client is told this send, each serialized once, whole and
/// as an update for the clients that hold it.
#[derive(Default)]
struct Serialized {
    bytes: Vec<u8>,
    kinds: Vec<Serialization>,
}

struct Serialization {
    kind: u16,
    whole: Range<usize>,
    update: Update,
}

/// What a client holding a kind is told of its change: nothing when its bytes came out as they
/// were, else the smaller of a patch over them and the whole.
enum Update {
    Unchanged,
    Whole,
    Patch(Range<usize>),
}

impl Serialized {
    fn changes<'a>(&'a self, told: &Told) -> impl Iterator<Item = Change<'a>> {
        let (whole, updated) = (told.whole, told.updated);
        self.kinds.iter().filter_map(move |serialization| {
            let kind = bit(serialization.kind as usize);
            let whole_bytes = &self.bytes[serialization.whole.clone()];
            if whole & kind != 0 {
                return Some(Change::whole(serialization.kind, whole_bytes));
            }
            if updated & kind == 0 {
                return None;
            }
            match &serialization.update {
                Update::Unchanged => None,
                Update::Whole => Some(Change::whole(serialization.kind, whole_bytes)),
                Update::Patch(patch) => Some(Change::patch(
                    serialization.kind,
                    &self.bytes[patch.clone()],
                )),
            }
        })
    }
}

fn summarize(
    world: &World,
    protocol: &Protocol,
    replicated: ComponentId,
    entity: Entity,
    last_run: Tick,
    this_run: Tick,
) -> Option<Summary> {
    let entity = world.get_entity(entity).ok()?;
    if !entity.contains_id(replicated) {
        return None;
    }
    let mut summary = Summary {
        present: 0,
        changed: 0,
    };
    for (kind, rule) in protocol.components.iter().enumerate() {
        let Some(ticks) = entity.get_change_ticks_by_id(rule.id) else {
            continue;
        };
        if rule
            .requires
            .is_some_and(|requires| !entity.contains_id(requires))
        {
            continue;
        }
        summary.present |= bit(kind);
        if !rule.once && ticks.is_changed(last_run, this_run) {
            summary.changed |= bit(kind);
        }
    }
    Some(summary)
}

/// Serializes the `kinds` of `entity`, each as an update over what it last went out as too,
/// and keeps what each goes out as now in `sent_as`.
fn serialize(
    world: &World,
    protocol: &Protocol,
    entity: Entity,
    kinds: Kinds,
    sent_as: &mut Vec<(u16, Vec<u8>)>,
) -> Serialized {
    let mut serialized = Serialized::default();
    if kinds == 0 {
        return serialized;
    }
    let entity = world.entity(entity);
    let mut patch = Vec::new();
    for kind in kinds_of(kinds) {
        let start = serialized.bytes.len();
        (protocol.components[kind as usize].serialize)(entity, &mut serialized.bytes);
        let whole = start..serialized.bytes.len();
        let bytes = &serialized.bytes[whole.clone()];
        let last = match sent_as.iter().position(|(sent, _)| *sent == kind) {
            Some(at) => &mut sent_as[at].1,
            None => {
                sent_as.push((kind, Vec::new()));
                &mut sent_as.last_mut().expect("just pushed").1
            }
        };
        let update = if last.as_slice() == bytes {
            Update::Unchanged
        } else if last.len() == bytes.len() {
            patch.clear();
            write_patch(last, bytes, &mut patch);
            if patch.len() < bytes.len() {
                Update::Patch(serialized.bytes.len()..serialized.bytes.len() + patch.len())
            } else {
                Update::Whole
            }
        } else {
            Update::Whole
        };
        last.clear();
        last.extend_from_slice(bytes);
        if let Update::Patch(_) = update {
            serialized.bytes.extend_from_slice(&patch);
        }
        serialized.kinds.push(Serialization {
            kind,
            whole,
            update,
        });
    }
    serialized
}

fn bit(kind: usize) -> Kinds {
    1 << kind
}

fn kinds_of(mut kinds: Kinds) -> impl Iterator<Item = u16> {
    std::iter::from_fn(move || {
        (kinds != 0).then(|| {
            let kind = kinds.trailing_zeros() as u16;
            kinds &= kinds - 1;
            kind
        })
    })
}
