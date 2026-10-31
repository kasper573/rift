use bevy_app::App;
use bevy_ecs::message::Message;
use bevy_ecs::prelude::*;
use bevy_replication::{Interest, Replicated, ReplicationInterest, ToClient};

use crate::core::chunk::ChunkIndex;
use crate::core::math::Pos;
use crate::core::units::Meters;
use crate::systems::area::{AreaId, AreaTag};
use crate::systems::mirror::AppMirrorExt;
use crate::systems::movement::{Position, position};
use crate::systems::player::{ClientId, PlayedBy, Players};
use crate::systems::script::condition::CompiledCondition;

pub const VIEW_DISTANCE: Meters = Meters(24.0);
// Compared against squared distance to avoid a per-pair `sqrt`. Exact: `VIEW_DISTANCE` is a perfect
// square in f32, so `d <= VIEW_DISTANCE` iff `d² <= VIEW_DISTANCE²` for all inputs.
const VIEW_DISTANCE_SQ: f32 = VIEW_DISTANCE.0 * VIEW_DISTANCE.0;

pub fn register(app: &mut App) {
    app.mirror::<Presence>();
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

/// What a client sees: everything replicated that stands nowhere, its own character, and what
/// stands in its character's area within `VIEW_DISTANCE` of it; of all those, only what is present
/// to its character. Only the client playing a character is told its private components.
#[derive(Default)]
pub struct ClientSight {
    unplaced: Vec<Entity>,
}

impl Interest for ClientSight {
    fn prepare(&mut self, world: &mut World) {
        self.unplaced = world
            .query_filtered::<Entity, (With<Replicated>, Without<Position>)>()
            .iter(world)
            .collect();
    }

    fn seen_by(&self, world: &World, client: Entity, seen: &mut Vec<Entity>) {
        let Some(view) = world
            .get::<ClientId>(client)
            .and_then(|&id| view(world, id))
        else {
            return;
        };
        let near = view
            .pos
            .into_iter()
            .flat_map(|pos| world.resource::<ChunkIndex>().around(pos, VIEW_DISTANCE));
        seen.extend(
            self.unplaced
                .iter()
                .copied()
                .chain(std::iter::once(view.focus))
                .chain(near)
                .filter(|&subject| view.sees(world, subject)),
        );
    }

    fn owns(&self, world: &World, client: Entity, entity: Entity) -> bool {
        world
            .get::<PlayedBy>(entity)
            .is_some_and(|played| world.get::<ClientId>(client) == Some(&played.client))
    }
}

/// The clients that see `entity`.
pub fn seen_by(world: &mut World, entity: Entity) -> Vec<Entity> {
    if position(world, entity).is_none() {
        return Vec::new();
    }
    let clients: Vec<(Entity, ClientId)> = clients(world);
    clients
        .into_iter()
        .filter(|&(_, id)| view(world, id).is_some_and(|view| view.sees(world, entity)))
        .map(|(client, _)| client)
        .collect()
}

/// The clients that play `player`.
pub fn private_viewers(world: &mut World, player: Entity) -> Vec<Entity> {
    let Some(owner) = world.get::<PlayedBy>(player).map(|owner| owner.client) else {
        return Vec::new();
    };
    clients(world)
        .into_iter()
        .filter(|&(_, client)| client == owner)
        .map(|(conn, _)| conn)
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

fn clients(world: &mut World) -> Vec<(Entity, ClientId)> {
    world.resource_scope(|world, mut sights: Mut<Sights>| {
        sights
            .0
            .iter(world)
            .map(|(entity, &id)| (entity, id))
            .collect()
    })
}

#[derive(Resource)]
pub(crate) struct Sights(QueryState<(Entity, &'static ClientId)>);

impl FromWorld for Sights {
    fn from_world(world: &mut World) -> Self {
        Sights(world.query())
    }
}

/// Where a client looks from: its character, where it stands.
struct View {
    focus: Entity,
    pos: Option<Pos<Meters>>,
    area: Option<AreaId>,
}

fn view(world: &World, client: ClientId) -> Option<View> {
    let focus = world.resource::<Players>().0.get(&client).copied()?;
    Some(View {
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
