use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_ecs::query::QueryState;
use bevy_replicon::prelude::{AppVisibilityExt, Replicated, SingleComponent, VisibilityFilter};
use bevy_replicon::server::visibility::client_visibility::ClientVisibility;
use bevy_replicon::server::visibility::filters_mask::FilterBit;
use bevy_replicon::server::visibility::registry::FilterRegistry;
use bevy_replicon::shared::replication::registry::ReplicationRegistry;

use crate::core::math::Pos;
use crate::core::tiling::Tiles;
use crate::systems::area::{self, AreaTag};
use crate::systems::item::Inventory;
use crate::systems::movement::{Position, position};
use crate::systems::player::{ClientId, Players};
use crate::systems::spectate::Spectators;

pub const VIEW_DISTANCE: Tiles = Tiles(24.0);
// Compared against squared distance to avoid a per-pair `sqrt`. Exact: `VIEW_DISTANCE` is a perfect
// square in f32, so `d <= VIEW_DISTANCE` iff `d² <= VIEW_DISTANCE²` for all inputs.
const VIEW_DISTANCE_SQ: f32 = VIEW_DISTANCE.0 * VIEW_DISTANCE.0;

pub fn register(app: &mut App) {
    app.add_visibility_filter::<OwnedBy>();
    app.init_resource::<RangeBit>();
}

type Clients = QueryState<(Entity, &'static ClientId)>;
type Subjects = QueryState<(Entity, &'static Position, Option<&'static AreaTag>), With<Replicated>>;

pub fn update(world: &mut World, clients_query: &mut Clients, subjects_query: &mut Subjects) {
    let clients: Vec<(Entity, ClientId)> = clients_query
        .iter(world)
        .map(|(entity, &id)| (entity, id))
        .collect();
    if clients.is_empty() {
        return;
    }
    let bit = world.resource::<RangeBit>().0;
    let subjects: Vec<Subject> = subjects_query
        .iter(world)
        .map(|(entity, position, tag)| Subject {
            entity,
            pos: position.pos,
            area: tag.map(|tag| tag.area),
        })
        .collect();
    for (client, id) in clients {
        let sight = sight(world, id);
        let Some(mut visibility) = world.get_mut::<ClientVisibility>(client) else {
            continue;
        };
        for subject in &subjects {
            visibility.set(subject.entity, bit, sees(sight.as_ref(), subject));
        }
    }
}

pub fn seen_by(world: &mut World, entity: Entity) -> Vec<Entity> {
    let Some(pos) = position(world, entity) else {
        return Vec::new();
    };
    let subject = Subject {
        entity,
        pos,
        area: world.get::<AreaTag>(entity).map(|tag| tag.area),
    };
    let clients: Vec<(Entity, ClientId)> = world
        .query::<(Entity, &ClientId)>()
        .iter(world)
        .map(|(entity, &id)| (entity, id))
        .collect();
    clients
        .into_iter()
        .filter(|&(_, id)| sees(sight(world, id).as_ref(), &subject))
        .map(|(client, _)| client)
        .collect()
}

#[derive(Component)]
#[component(immutable)]
pub struct OwnedBy(pub ClientId);

impl VisibilityFilter for OwnedBy {
    type ClientComponent = ClientId;
    type Scope = SingleComponent<Inventory>;

    fn is_visible(&self, _: Entity, client: Option<&ClientId>) -> bool {
        client == Some(&self.0)
    }
}

#[derive(Resource, Clone, Copy)]
struct RangeBit(FilterBit);

impl FromWorld for RangeBit {
    fn from_world(world: &mut World) -> Self {
        let bit = world.resource_scope(|world, mut filters: Mut<FilterRegistry>| {
            world.resource_scope(|world, mut registry: Mut<ReplicationRegistry>| {
                filters.register_scope::<Entity>(world, &mut registry)
            })
        });
        RangeBit(bit)
    }
}

struct Subject {
    entity: Entity,
    pos: Pos<Tiles>,
    area: Option<area::Id>,
}

struct Sight {
    focus: Entity,
    pos: Option<Pos<Tiles>>,
    area: Option<area::Id>,
}

fn sight(world: &World, client: ClientId) -> Option<Sight> {
    let player = match world.resource::<Spectators>().0.get(&client) {
        Some(spectator) => spectator.watching()?,
        None => client,
    };
    let focus = world.resource::<Players>().0.get(&player).copied()?;
    Some(Sight {
        focus,
        pos: position(world, focus),
        area: world.get::<AreaTag>(focus).map(|tag| tag.area),
    })
}

fn sees(sight: Option<&Sight>, subject: &Subject) -> bool {
    let Some(sight) = sight else {
        return false;
    };
    if subject.entity == sight.focus {
        return true;
    }
    let (Some(pos), Some(area)) = (sight.pos, sight.area) else {
        return false;
    };
    subject.area == Some(area) && (pos - subject.pos).square_length() <= VIEW_DISTANCE_SQ
}
