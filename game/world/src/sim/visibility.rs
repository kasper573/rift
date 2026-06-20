//! Visibility rules: players see nearby entities in their area; spectators see their watched player
//! and the full roster for UI; spectator anchors stay invisible; inventories replicate only to owners.

use std::collections::HashSet;

use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_replicon::prelude::{
    AppVisibilityExt, AuthorizedClient, Replicated, SingleComponent, VisibilityFilter,
};
use bevy_replicon::server::visibility::client_visibility::ClientVisibility;
use bevy_replicon::server::visibility::filters_mask::FilterBit;
use bevy_replicon::server::visibility::registry::FilterRegistry;
use bevy_replicon::shared::replication::registry::ReplicationRegistry;

use super::player::Players;
use super::spectate::Spectators;
use crate::area::AreaDef;
use crate::math::{Pos, Tiles};
use crate::protocol::{AreaTag, ClientId, Inventory, Position, Spectate, position};
use crate::table::Id;

pub const VIEW_DISTANCE: Tiles = Tiles(24.0);

pub fn register(app: &mut App) {
    app.add_visibility_filter::<OwnedBy>();
    app.init_resource::<RangeBit>();
}

pub fn update(world: &mut World) {
    let bit = world.resource::<RangeBit>().0;
    let players: HashSet<Entity> = world.resource::<Players>().0.values().copied().collect();
    let clients: Vec<(Entity, ClientId)> = world
        .query::<(Entity, &ClientId)>()
        .iter(world)
        .map(|(entity, &id)| (entity, id))
        .collect();
    let subjects: Vec<Subject> = world
        .query_filtered::<(Entity, &Position, Option<&AreaTag>, Has<Spectate>), With<Replicated>>()
        .iter(world)
        .map(|(entity, position, tag, anchor)| Subject {
            entity,
            pos: position.pos,
            area: tag.map(|tag| tag.area),
            anchor,
        })
        .collect();
    for (client, id) in clients {
        let sight = sight(world, id);
        let Some(mut visibility) = world.get_mut::<ClientVisibility>(client) else {
            continue;
        };
        for subject in &subjects {
            visibility.set(subject.entity, bit, sees(sight.as_ref(), &players, subject));
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
        anchor: world.get::<Spectate>(entity).is_some(),
    };
    let players: HashSet<Entity> = world.resource::<Players>().0.values().copied().collect();
    let clients: Vec<(Entity, ClientId)> = world
        .query::<(Entity, &ClientId)>()
        .iter(world)
        .map(|(entity, &id)| (entity, id))
        .collect();
    clients
        .into_iter()
        .filter(|&(_, id)| sees(sight(world, id).as_ref(), &players, &subject))
        .map(|(client, _)| client)
        .collect()
}

#[derive(Component)]
#[component(immutable)]
pub struct OwnedBy(pub Entity);

impl VisibilityFilter for OwnedBy {
    type ClientComponent = AuthorizedClient;
    type Scope = SingleComponent<Inventory>;

    fn is_visible(&self, client: Entity, _: Option<&AuthorizedClient>) -> bool {
        self.0 == client
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
    area: Option<Id<AreaDef>>,
    anchor: bool,
}

struct Sight {
    focus: Entity,
    pos: Option<Pos<Tiles>>,
    area: Option<Id<AreaDef>>,
    spectating: bool,
}

fn sight(world: &World, client: ClientId) -> Option<Sight> {
    let player = world.resource::<Players>().0.get(&client).copied();
    let anchor = world.resource::<Spectators>().0.get(&client).copied();
    let focus = player.or(anchor)?;
    Some(Sight {
        focus,
        pos: position(world, focus),
        area: world.get::<AreaTag>(focus).map(|tag| tag.area),
        spectating: anchor.is_some(),
    })
}

fn sees(sight: Option<&Sight>, players: &HashSet<Entity>, subject: &Subject) -> bool {
    let Some(sight) = sight else {
        return false;
    };
    if subject.entity == sight.focus {
        return true;
    }
    let (Some(pos), Some(area)) = (sight.pos, sight.area) else {
        return false;
    };
    if subject.area != Some(area) {
        return false;
    }
    (!subject.anchor && pos.distance_to(subject.pos) <= VIEW_DISTANCE.0)
        || (sight.spectating && players.contains(&subject.entity))
}
