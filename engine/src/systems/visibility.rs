use bevy_app::App;
use bevy_ecs::message::Message;
use bevy_ecs::prelude::*;
use bevy_replication::{Interest, Replicated, ReplicationInterest, ToClient};

use crate::core::chunk::{CHUNK_SIDE, Chunk, ChunkIndex};
use crate::core::math::Pos;
use crate::core::tick::Planner;
use crate::core::units::{MeterPos, Meters};
use crate::systems::area::{self, AreaId, AreaTag};
use crate::systems::movement::{Position, position};
use crate::systems::player::{ClientId, Connected, PlayedBy, Players};
use crate::systems::script::condition::CompiledCondition;

pub const VIEW_DISTANCE: Meters = Meters(24.0);
// Compared against squared distance to avoid a per-pair `sqrt`. Exact: `VIEW_DISTANCE` is a perfect
// square in f32, so `d <= VIEW_DISTANCE` iff `d² <= VIEW_DISTANCE²` for all inputs.
const VIEW_DISTANCE_SQ: f32 = VIEW_DISTANCE.0 * VIEW_DISTANCE.0;

pub fn register(app: &mut App) {
    app.insert_resource(ReplicationInterest(Box::new(ClientSight::default())));
}

#[derive(Component, Clone)]
pub enum Presence {
    For(ClientId),
    When(CompiledCondition),
}

pub fn present(world: &World, subject: Entity, viewer: Entity) -> bool {
    match world.get::<Presence>(subject) {
        None => true,
        Some(Presence::For(client)) => world
            .get::<PlayedBy>(viewer)
            .is_some_and(|owner| owner.client == *client),
        Some(Presence::When(shown)) => shown.holds(world, viewer),
    }
}

pub fn coexist(world: &World, a: Entity, b: Entity) -> bool {
    present(world, a, b) && present(world, b, a)
}

/// Whether `subject` is among what `viewer` is shown: present to each other, in the same area
/// within [`VIEW_DISTANCE`]. What a player may aim at, reach for or pick up, since their client
/// shows nothing else; a request for anything farther didn't come from what they see.
pub fn in_sight(world: &World, viewer: Entity, subject: Entity) -> bool {
    coexist(world, viewer, subject)
        && area::same_area(world, viewer, subject)
        && matches!(
            (position(world, viewer), position(world, subject)),
            (Some(at), Some(there)) if at.distance(there) <= VIEW_DISTANCE
        )
}

/// What a client sees: everything replicated that stands nowhere, its own character, and what
/// stands in its character's area within `VIEW_DISTANCE` of it; of all those, only what is present
/// to its character. Only the client playing a character is told its private components.
#[derive(Default)]
pub struct ClientSight {
    unplaced: Vec<Entity>,
    /// Every replicated entity that stands somewhere, by its index, as a view checks it: read
    /// once per send rather than once per client that looks its way.
    standing: Vec<Option<Standing>>,
}

/// What a view checks of an entity standing in a chunk.
#[derive(Clone, Copy)]
struct Standing {
    entity: Entity,
    at: Pos<Meters>,
    area: Option<AreaId>,
    shown: Shown,
}

#[derive(Clone, Copy)]
enum Shown {
    ToAll,
    To(ClientId),
    /// On a condition of the viewer, asked of the world.
    When,
}

impl Interest for ClientSight {
    fn prepare(&mut self, world: &mut World) {
        self.unplaced = world
            .query_filtered::<Entity, (With<Replicated>, Without<Position>)>()
            .iter(world)
            .collect();
        self.standing.clear();
        self.standing.resize(world.entities().len() as usize, None);
        let mut placed = world.query_filtered::<
            (Entity, &Position, Option<&AreaTag>, Option<&Presence>),
            With<Replicated>,
        >();
        for (entity, position, tag, presence) in placed.iter(world) {
            self.standing[entity.index_u32() as usize] = Some(Standing {
                entity,
                at: position.pos,
                area: tag.map(|tag| tag.area),
                shown: match presence {
                    None => Shown::ToAll,
                    Some(Presence::For(client)) => Shown::To(*client),
                    Some(Presence::When(_)) => Shown::When,
                },
            });
        }
    }

    /// What stands in the chunks around the character, as the chunks are filed this tick.
    fn seen_by(&self, world: &World, client: Entity, seen: &mut Vec<Entity>) {
        let Some(view) = world
            .get::<ClientId>(client)
            .and_then(|&id| view(world, id))
        else {
            return;
        };
        seen.extend(
            self.unplaced
                .iter()
                .copied()
                .chain(std::iter::once(view.focus))
                .filter(|&subject| view.sees(world, subject)),
        );
        let (Some(pos), Some(area)) = (view.pos, view.area) else {
            return;
        };
        let chunks = world.resource::<ChunkIndex>();
        for chunk in Chunk::around(pos, VIEW_DISTANCE) {
            seen.extend(
                chunks
                    .in_chunk(chunk)
                    .iter()
                    .copied()
                    .filter(|&subject| subject != view.focus)
                    .filter(
                        |&subject| match self.standing.get(subject.index_u32() as usize) {
                            Some(Some(standing)) if standing.entity == subject => {
                                standing.area == Some(area)
                                    && (pos - standing.at).square_length() <= VIEW_DISTANCE_SQ
                                    && match standing.shown {
                                        Shown::ToAll => true,
                                        Shown::To(client) => view.client == client,
                                        Shown::When => present(world, subject, view.focus),
                                    }
                            }
                            _ => view.sees(world, subject),
                        },
                    ),
            );
        }
    }

    fn owns(&self, world: &World, client: Entity, entity: Entity) -> bool {
        world
            .get::<PlayedBy>(entity)
            .is_some_and(|played| world.get::<ClientId>(client) == Some(&played.client))
    }
}

/// The clients that see `entity`: those whose characters stand in the chunks around it, as the
/// chunks were filed last tick (hence a chunk of margin), and see it where it stands now.
pub fn seen_by(world: &World, entity: Entity) -> Vec<Entity> {
    let Some(at) = position(world, entity) else {
        return Vec::new();
    };
    let connected = world.resource::<Connected>();
    world
        .resource::<ChunkIndex>()
        .around(at, Meters(VIEW_DISTANCE.0 + CHUNK_SIDE.0))
        .filter_map(|near| world.get::<PlayedBy>(near).map(|played| played.client))
        .filter(|&client| view(world, client).is_some_and(|view| view.sees(world, entity)))
        .filter_map(|client| connected.connection(client))
        .collect()
}

/// The clients that play `player`.
pub fn private_viewers(world: &World, player: Entity) -> Vec<Entity> {
    let Some(owner) = world.get::<PlayedBy>(player).map(|owner| owner.client) else {
        return Vec::new();
    };
    world
        .resource::<Connected>()
        .connection(owner)
        .into_iter()
        .collect()
}

pub fn send_private<M: Message + Clone>(world: &mut World, player: Entity, message: M) {
    for conn in private_viewers(world, player) {
        send_to(world, conn, message.clone());
    }
}

pub fn send_to<M: Message>(world: &mut World, conn: Entity, message: M) {
    world.write_message(ToClient {
        client: conn,
        message,
    });
}

/// Where a client looks from: its character, where it stands.
struct View {
    client: ClientId,
    focus: Entity,
    pos: Option<Pos<Meters>>,
    area: Option<AreaId>,
}

fn view(world: &World, client: ClientId) -> Option<View> {
    let focus = world.resource::<Players>().0.get(&client).copied()?;
    Some(View {
        client,
        focus,
        pos: position(world, focus),
        area: world.get::<AreaTag>(focus).map(|tag| tag.area),
    })
}

impl View {
    fn sees(&self, world: &World, subject: Entity) -> bool {
        if subject == self.focus {
            return true;
        }
        if world.get::<Replicated>(subject).is_none() || !present(world, subject, self.focus) {
            return false;
        }
        let Some(at) = position(world, subject) else {
            return true;
        };
        let (Some(pos), Some(area)) = (self.pos, self.area) else {
            return false;
        };
        world.get::<AreaTag>(subject).map(|tag| tag.area) == Some(area)
            && (pos - at).square_length() <= VIEW_DISTANCE_SQ
    }
}

/// Tells every client what changed in its sight, each replication tick, planned beside the rest
/// of the tick from the world as the tick found it.
#[derive(Default)]
pub struct SendSights(bevy_replication::Sender);

impl Planner for SendSights {
    fn prepare(&mut self, world: &mut World) {
        if crate::systems::replication_tick(world) {
            self.0.prepare(world);
        }
    }

    fn plan(&mut self, world: &World) {
        self.0.plan(world);
    }

    fn apply(&mut self, world: &mut World) {
        self.0.apply(world);
    }
}
