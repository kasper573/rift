//! An entity's name across every world that holds it: the same in each world of the server and on
//! every client, so what one world hands another, or what a client comes to see from another world,
//! is known as itself.

use std::sync::Arc;

use bevy_app::App;
use bevy_ecs::entity::EntityHashMap;
use bevy_ecs::lifecycle::HookContext;
use bevy_ecs::prelude::*;
use bevy_ecs::query::QueryState;
use bevy_ecs::world::DeferredWorld;
use bevy_platform::collections::HashMap;
use bevy_replicon::prelude::{AppRuleExt, Replicated};
use serde::{Deserialize, Serialize};

#[derive(
    Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[component(immutable, on_insert = GlobalId::name, on_remove = GlobalId::unname)]
pub struct GlobalId(u64);

/// Where a world's names come from: its own pool, told apart from every other world's by the
/// world's ordinal in the high bits, so no two worlds name alike and none waits on another.
#[derive(Resource)]
pub struct Names {
    world: u64,
    next: u64,
}

impl Names {
    pub fn for_world(ordinal: u64) -> Names {
        assert!(
            ordinal < 1 << WORLD_BITS,
            "world ordinal {ordinal} out of the name space"
        );
        Names {
            world: ordinal,
            next: 1,
        }
    }

    /// A name no entity of this process has had.
    pub fn give(&mut self) -> GlobalId {
        let id = GlobalId(self.world << (64 - WORLD_BITS) | self.next);
        self.next += 1;
        id
    }
}

const WORLD_BITS: u32 = 24;

pub fn register(app: &mut App) {
    app.init_resource::<Named>();
    app.replicate_once::<GlobalId>();
}

/// Names every replicated entity that has none yet, before the tick's replication tells clients
/// of it.
pub fn name_replicated(
    world: &mut World,
    unnamed: &mut QueryState<Entity, (With<Replicated>, Without<GlobalId>)>,
) {
    let fresh: Vec<Entity> = unnamed.iter(world).collect();
    for entity in fresh {
        let id = world.resource_mut::<Names>().give();
        world.entity_mut(entity).insert(id);
    }
}

/// Which entity of this world bears each name, and which name each bears, kept by the names
/// themselves as they come and go. Both maps hand out shared snapshots, copied only once a name
/// has changed since the last.
#[derive(Resource, Default)]
pub struct Named {
    by_entity: Arc<EntityHashMap<GlobalId>>,
    by_name: Arc<HashMap<GlobalId, Entity>>,
}

impl Named {
    pub fn contains(&self, id: GlobalId) -> bool {
        self.by_name.contains_key(&id)
    }

    pub fn get(&self, id: GlobalId) -> Option<Entity> {
        self.by_name.get(&id).copied()
    }

    pub fn name(&self, entity: Entity) -> Option<GlobalId> {
        self.by_entity.get(&entity).copied()
    }

    pub fn by_entity(&self) -> Arc<EntityHashMap<GlobalId>> {
        self.by_entity.clone()
    }

    pub fn by_name(&self) -> Arc<HashMap<GlobalId, Entity>> {
        self.by_name.clone()
    }
}

impl GlobalId {
    fn name(mut world: DeferredWorld, context: HookContext) {
        let Some(&id) = world.get::<GlobalId>(context.entity) else {
            return;
        };
        let Some(mut named) = world.get_resource_mut::<Named>() else {
            return;
        };
        Arc::make_mut(&mut named.by_entity).insert(context.entity, id);
        Arc::make_mut(&mut named.by_name).insert(id, context.entity);
    }

    fn unname(mut world: DeferredWorld, context: HookContext) {
        let Some(&id) = world.get::<GlobalId>(context.entity) else {
            return;
        };
        let Some(mut named) = world.get_resource_mut::<Named>() else {
            return;
        };
        Arc::make_mut(&mut named.by_entity).remove(&context.entity);
        Arc::make_mut(&mut named.by_name).remove(&id);
    }
}
