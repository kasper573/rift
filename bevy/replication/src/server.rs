use std::ops::Range;
use std::sync::OnceLock;

use bevy_app::{App, Plugin, PostUpdate, PreUpdate};
use bevy_ecs::change_detection::Tick;
use bevy_ecs::component::ComponentId;
use bevy_ecs::prelude::*;
use rayon::prelude::*;

use crate::protocol::{Kinds, Protocol};
use crate::wire::{
    Change, ClientFrame, EntityChangesOut, Part, ServerFrameOut, WireEntity, write_patch,
};

/// Replicates what the protocol declares of every `Replicated` entity to each `ClientLink`, as
/// far as the interest lets each see, and reads what clients send.
#[derive(Default)]
pub struct ServerReplicationPlugin {
    /// Who runs the sends.
    pub sending: Sending,
}

/// Who runs the sends.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Sending {
    /// The plugin, every update, in `ServerSystems::Send`.
    #[default]
    EveryUpdate,
    /// The host, through a `Sender` of its own, wherever its schedule sends.
    ByHost,
}

impl Plugin for ServerReplicationPlugin {
    fn build(&self, app: &mut App) {
        app.world_mut().register_component::<Replicated>();
        app.init_resource::<Protocol>()
            .init_resource::<ReplicationInterest>()
            .init_resource::<LastSend>()
            .init_resource::<LastSent>()
            .add_systems(PreUpdate, receive.in_set(ServerSystems::Receive))
            .add_systems(PostUpdate, queue_messages);
        if self.sending == Sending::EveryUpdate {
            app.add_systems(
                PostUpdate,
                send.in_set(ServerSystems::Send).after(queue_messages),
            );
        }
    }
}

/// Where the server reads what clients sent, and where it sends: an app that sends less often than
/// every update gives `Send` a run condition; messages written in between wait for the next send.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ServerSystems {
    /// Reads what every `ClientLink` received, in `PreUpdate`.
    Receive,
    /// Hands every `ClientLink` its frame, in `PostUpdate`.
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
    /// Every entity the client holds, in entity order, with the component kinds it holds of each.
    known: Vec<(Entity, Kinds)>,
    messages: Vec<(u16, Vec<u8>)>,
    misbehaved: bool,
}

impl ClientLink {
    /// Takes bytes the client sent, as the transport delivered them.
    pub fn receive(&mut self, bytes: Vec<u8>) {
        self.received.push(bytes);
    }

    /// What the transport is to send the client, each piece delivered as one.
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

/// The bytes every replicated entity some client sees last went out as, each under its entity's
/// index.
#[derive(Resource, Default)]
struct LastSent(Vec<Option<Sent>>);

/// The bytes an entity last went out as, kind by kind: what every client holding that kind of it
/// holds, as each was told every change since.
struct Sent {
    entity: Entity,
    kinds: Vec<(u16, Vec<u8>)>,
}

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

/// How finely a pass over many clients or entities is cut into jobs: a few per core, so no core
/// idles behind one slow job and none is spent handing jobs out.
const JOBS_PER_THREAD: usize = 2;

fn grain(len: usize) -> usize {
    len.div_ceil(JOBS_PER_THREAD * rayon::current_num_threads())
        .max(1)
}

fn send(world: &mut World, mut sender: Local<Sender>) {
    sender.prepare(world);
    sender.plan(world);
    sender.apply(world);
}

/// A send in three steps, for a host that plans it beside work of its own: `prepare` takes from
/// the world what the send holds of its clients, `plan` works out every client's frame from the
/// world as it stands, all clients at once on every core (what each sees, what it must be told
/// of each, and the frame of it; what each seen entity is there as, what changed of it, and its
/// bytes, worked out once by whichever client reaches it first), and `apply` hands the frames to
/// the links. Run in turn, the three are one send; a plan or an apply without a prepare before
/// it does nothing.
#[derive(Default)]
pub struct Sender {
    links: Option<QueryState<(Entity, &'static mut ClientLink)>>,
    ticks: Option<(Tick, Tick)>,
    clients: Vec<Held>,
    last_sent: Vec<Option<Sent>>,
    subjects: Vec<OnceLock<Option<Subject>>>,
    frames: Vec<Option<Vec<u8>>>,
}

impl Sender {
    /// Takes from the world what the send holds of its clients, and readies the interest.
    pub fn prepare(&mut self, world: &mut World) {
        let this_run = world.change_tick();
        let last_run = std::mem::replace(&mut world.resource_mut::<LastSend>().0, this_run);
        world.resource_scope(|world, mut interest: Mut<ReplicationInterest>| {
            interest.0.prepare(world);
        });
        self.last_sent = std::mem::take(&mut world.resource_mut::<LastSent>().0);
        let links = self.links.get_or_insert_with(|| world.query());
        self.clients = links
            .iter_mut(world)
            .map(|(client, mut link)| Held {
                client,
                fresh: (!std::mem::replace(&mut link.started, true)).then_some(link.epoch),
                known: std::mem::take(&mut link.known),
                messages: std::mem::take(&mut link.messages),
            })
            .collect();
        self.ticks = Some((last_run, this_run));
    }

    /// Works out every client's frame from the world as it stands, on every core.
    pub fn plan(&mut self, world: &World) {
        let Some((last_run, this_run)) = self.ticks else {
            return;
        };
        let protocol = world.resource::<Protocol>();
        let interest = &*world.resource::<ReplicationInterest>().0;
        let replicated = world
            .component_id::<Replicated>()
            .expect("the plugin registers Replicated");
        let private = protocol
            .components
            .iter()
            .enumerate()
            .filter(|(_, rule)| rule.private)
            .fold(0, |mask, (kind, _)| mask | bit(kind));

        let indices = world.entities().len() as usize;
        self.last_sent.resize_with(indices, || None);
        self.subjects.resize_with(indices, OnceLock::new);
        let last_sent = &self.last_sent;
        let subjects = &self.subjects;
        let subject = |entity: Entity| -> Option<&Subject> {
            let index = entity.index_u32() as usize;
            subjects[index]
                .get_or_init(|| {
                    let summary =
                        summarize(world, protocol, replicated, entity, last_run, this_run)?;
                    Some(Subject {
                        entity,
                        summary,
                        kinds: (0..summary.present.count_ones())
                            .map(|_| OnceLock::new())
                            .collect(),
                    })
                })
                .as_ref()
        };

        let clients = &mut self.clients;
        let client_grain = grain(clients.len());
        self.frames = clients
            .par_iter_mut()
            .with_min_len(client_grain)
            .map_init(Scratch::default, |scratch, held| {
                let Scratch {
                    seen,
                    known,
                    left,
                    changes,
                    entities,
                } = scratch;
                seen.clear();
                interest.seen_by(world, held.client, seen);
                seen.sort_unstable();
                seen.dedup();
                known.clear();
                left.clear();
                changes.clear();
                entities.clear();
                let mut held_before = held.known.iter().peekable();
                for &entity in seen.iter() {
                    while let Some(&&(before, _)) = held_before.peek()
                        && before < entity
                    {
                        left.push(WireEntity::from(before));
                        held_before.next();
                    }
                    let holds = held_before
                        .next_if(|&&(before, _)| before == entity)
                        .map(|&(_, holds)| holds);
                    let Some(subject) = subject(entity) else {
                        left.extend(holds.map(|_| WireEntity::from(entity)));
                        continue;
                    };
                    let mut want = subject.summary.present;
                    if want & private != 0 && !interest.owns(world, held.client, entity) {
                        want &= !private;
                    }
                    known.push((entity, want));
                    let told = match holds {
                        None => Told {
                            arrives: true,
                            whole: want,
                            updated: 0,
                            removed: 0,
                        },
                        Some(holds) => Told {
                            arrives: false,
                            whole: want & !holds,
                            updated: want & holds & subject.summary.changed,
                            removed: holds & !want,
                        },
                    };
                    if !told.arrives && told.whole | told.updated | told.removed == 0 {
                        continue;
                    }
                    let start = changes.len();
                    changes.extend(subject.changes(world, protocol, last_sent, &told));
                    changes.extend(kinds_of(told.removed).map(|kind| Change::whole(kind, &[])));
                    if told.arrives || changes.len() > start {
                        entities.push((entity, start..changes.len()));
                    }
                }
                left.extend(held_before.map(|&(before, _)| WireEntity::from(before)));
                held.known.clear();
                held.known.extend_from_slice(known);
                let entities: Vec<EntityChangesOut> = entities
                    .iter()
                    .map(|(entity, range)| EntityChangesOut {
                        entity: WireEntity::from(*entity),
                        changes: &changes[range.clone()],
                    })
                    .collect();
                let messages: Vec<Part> = held
                    .messages
                    .iter()
                    .map(|(kind, bytes)| Part {
                        kind: *kind,
                        bytes: bytes.as_slice(),
                    })
                    .collect();
                let frame = ServerFrameOut {
                    fresh: held.fresh,
                    left,
                    entities: &entities,
                    messages: &messages,
                };
                (!frame.is_empty())
                    .then(|| postcard::to_allocvec(&frame).expect("a server frame serializes"))
            })
            .collect();

        self.subjects
            .par_iter_mut()
            .zip(self.last_sent.par_iter_mut())
            .with_min_len(grain(indices))
            .for_each(|(slot, last_sent)| {
                let Some(subject) = slot.take().flatten() else {
                    *last_sent = None;
                    return;
                };
                let last = last_sent
                    .take()
                    .filter(|sent| sent.entity == subject.entity)
                    .map(|sent| sent.kinds)
                    .unwrap_or_default();
                let mut last = last.into_iter().peekable();
                let mut kinds = Vec::with_capacity(subject.kinds.len());
                for (kind, serialization) in kinds_of(subject.summary.present).zip(subject.kinds) {
                    let bytes = match serialization.into_inner() {
                        Some(serialization) => Some(serialization.whole),
                        None => {
                            while last.peek().is_some_and(|(sent, _)| *sent < kind) {
                                last.next();
                            }
                            last.next_if(|(sent, _)| *sent == kind)
                                .map(|(_, bytes)| bytes)
                        }
                    };
                    if let Some(bytes) = bytes {
                        kinds.push((kind, bytes));
                    }
                }
                *last_sent = (!kinds.is_empty()).then_some(Sent {
                    entity: subject.entity,
                    kinds,
                });
            });
    }

    /// Hands the planned frames to the links.
    pub fn apply(&mut self, world: &mut World) {
        if self.ticks.take().is_none() {
            return;
        }
        world.resource_mut::<LastSent>().0 = std::mem::take(&mut self.last_sent);
        for (held, frame) in self.clients.drain(..).zip(self.frames.drain(..)) {
            let Some(mut link) = world.get_mut::<ClientLink>(held.client) else {
                continue;
            };
            link.known = held.known;
            link.outgoing.extend(frame);
        }
    }
}

/// What a job of the send keeps from one client's frame to the next, so a frame allocates
/// nothing but its bytes.
#[derive(Default)]
struct Scratch<'a> {
    seen: Vec<Entity>,
    known: Vec<(Entity, Kinds)>,
    left: Vec<WireEntity>,
    changes: Vec<Change<'a>>,
    entities: Vec<(Entity, Range<usize>)>,
}

/// What a send holds of a client while it works out what to tell it.
struct Held {
    client: Entity,
    fresh: Option<u32>,
    known: Vec<(Entity, Kinds)>,
    messages: Vec<(u16, Vec<u8>)>,
}

/// An entity some client sees this send, as every client that sees it is told of it: each of
/// its kinds serialized once, by whichever client first needs it.
struct Subject {
    entity: Entity,
    summary: Summary,
    /// One per present kind, in kind order.
    kinds: Vec<OnceLock<Serialization>>,
}

impl Subject {
    /// What to tell a client of the entity, each kind serialized as the first client needing it
    /// asks, over the bytes `last_sent` holds of it.
    fn changes<'a>(
        &'a self,
        world: &'a World,
        protocol: &'a Protocol,
        last_sent: &'a [Option<Sent>],
        told: &Told,
    ) -> impl Iterator<Item = Change<'a>> {
        let (whole, updated) = (told.whole, told.updated);
        kinds_of(whole | updated).filter_map(move |kind| {
            let serialization = self.serialization(world, protocol, last_sent, kind);
            if whole & bit(kind as usize) != 0 {
                return Some(Change::whole(kind, &serialization.whole));
            }
            match &serialization.update {
                Update::Unchanged => None,
                Update::Whole => Some(Change::whole(kind, &serialization.whole)),
                Update::Patch(patch) => Some(Change::patch(kind, patch)),
            }
        })
    }

    fn serialization(
        &self,
        world: &World,
        protocol: &Protocol,
        last_sent: &[Option<Sent>],
        kind: u16,
    ) -> &Serialization {
        let rank = (self.summary.present & (bit(kind as usize) - 1)).count_ones() as usize;
        self.kinds[rank].get_or_init(|| {
            let last = last_sent[self.entity.index_u32() as usize]
                .as_ref()
                .filter(|sent| sent.entity == self.entity)
                .and_then(|sent| sent.kinds.iter().find(|(sent, _)| *sent == kind))
                .map(|(_, bytes)| bytes.as_slice());
            serialize(world, protocol, self.entity, kind, last)
        })
    }
}

#[derive(Clone, Copy)]
struct Summary {
    /// The kinds the entity has, of those whose rules let them go out now.
    present: Kinds,
    /// The kinds among them that changed since the last send.
    changed: Kinds,
}

/// What to tell a client of one entity: whether it comes into the client's sight; the kinds the
/// client doesn't hold, told whole; those it holds that changed; and those it holds but may no
/// longer.
struct Told {
    arrives: bool,
    whole: Kinds,
    updated: Kinds,
    removed: Kinds,
}

/// One component of an entity as it goes out this send: whole, and as an update for the clients
/// that hold it.
struct Serialization {
    whole: Vec<u8>,
    update: Update,
}

/// What a client holding a kind is told of its change: nothing when its bytes came out as they
/// were, else the smaller of a patch over them and the whole.
enum Update {
    Unchanged,
    Whole,
    Patch(Vec<u8>),
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

/// Serializes the `kind` of `entity`, whole and as an update over the bytes it last went out as.
fn serialize(
    world: &World,
    protocol: &Protocol,
    entity: Entity,
    kind: u16,
    last: Option<&[u8]>,
) -> Serialization {
    let mut whole = Vec::new();
    (protocol.components[kind as usize].serialize)(world.entity(entity), &mut whole);
    let update = match last {
        Some(last) if last == whole => Update::Unchanged,
        Some(last) if last.len() == whole.len() => {
            let mut patch = Vec::new();
            write_patch(last, &whole, &mut patch);
            if patch.len() < whole.len() {
                Update::Patch(patch)
            } else {
                Update::Whole
            }
        }
        _ => Update::Whole,
    };
    Serialization { whole, update }
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
