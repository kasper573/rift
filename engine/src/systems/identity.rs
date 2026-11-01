//! An entity's name across every world that holds it: the same in each world of the server and on
//! every client, so what a client comes to see from another world is known as itself.

use bevy_app::{App, Plugin, PreUpdate};
use bevy_ecs::component::ComponentId;
use bevy_ecs::prelude::*;
use bevy_ecs::query::QueryState;
use bevy_ecs::schedule::common_conditions::resource_removed;
use bevy_ecs::world::EntityWorldMut;
use bevy_platform::collections::HashMap;
use bevy_replication::{
    AppReplicationExt, ClientSystems, Remote, Replicated, ReplicatedDespawn, ServerLink,
};
use serde::{Deserialize, Serialize};

use crate::core::render::transition::WorldViewSystems;
use crate::core::time::{self, Seconds};
use crate::systems::player::session;
use crate::systems::player::{ARRIVAL_PATIENCE, PlayedBy};

#[derive(
    Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[component(immutable)]
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

/// A client switches worlds when its character travels to another area: the
/// world it was in despawns everything it saw, and the next replicates its own entities, under
/// the same names where they stand for the same things. While the switch lasts, what the server
/// despawns stays as it was shown, and what the next world replicates under a name shown that way
/// takes over its depiction: the figure, the played-back position, everything the client made of
/// it. The player's own character is among them, so the view carries on across the switch.
/// Outside a switch a despawn is a despawn, that frame.
pub struct ClientIdentityPlugin;

impl Plugin for ClientIdentityPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Switching>()
            .insert_resource(ReplicatedDespawn(vanish))
            .add_systems(
                PreUpdate,
                (
                    settle
                        .after(ClientSystems::Receive)
                        .before(WorldViewSystems),
                    forget_lingering.run_if(resource_removed::<ServerLink>),
                ),
            );
    }
}

/// What a server's despawn does on a client: a named entity keeps its depiction under its name
/// in a stand-in of its own until the frame settles, any other goes at once.
pub fn vanish(mut entity: EntityWorldMut) {
    if entity.get::<GlobalId>().is_none() {
        entity.despawn();
        return;
    }
    let mine = entity
        .get::<PlayedBy>()
        .is_some_and(|played| Some(played.client) == session::my_id(entity.world()));
    let children: Vec<Entity> = entity
        .get::<Children>()
        .map(|children| children.iter().collect())
        .unwrap_or_default();
    let stand_in = entity.world_scope(|world| world.spawn(Lingering).id());
    entity.clone_with_opt_out(stand_in, |builder| {
        builder
            .move_components(true)
            .deny::<(Remote, Children, ChildOf)>();
    });
    entity.world_scope(|world| {
        for child in children {
            world.entity_mut(child).insert(ChildOf(stand_in));
        }
        if mine {
            let now = time::uptime(world);
            world.resource_mut::<Switching>().0.get_or_insert(now);
        }
    });
    entity.despawn();
}

/// A despawned entity kept as it was shown while the client switches worlds, for the entity the
/// next world replicates under its name to take over.
#[derive(Component)]
pub struct Lingering;

/// Since when the client's own character has been gone, while the next world's hasn't come.
#[derive(Resource, Default)]
struct Switching(Option<Seconds>);

/// The named entities a world has just replicated, which may take over a stand-in's depiction.
type Arrived = QueryState<(Entity, &'static GlobalId), (Added<GlobalId>, Without<Lingering>)>;

/// Hands every lingering entity's depiction to the entity that came back under its name, then
/// lets the rest go once the switch is over: when the client's own character is back, or it has
/// waited as long as an arrival is waited for, or no switch was on to begin with.
fn settle(
    world: &mut World,
    arrived: &mut Arrived,
    lingering: &mut QueryState<(Entity, &GlobalId), With<Lingering>>,
    played: &mut QueryState<&PlayedBy, Without<Lingering>>,
) {
    let stand_ins: HashMap<GlobalId, Entity> = lingering
        .iter(world)
        .map(|(entity, &name)| (name, entity))
        .collect();
    if stand_ins.is_empty() {
        return;
    }
    let adoptions: Vec<(Entity, Entity)> = arrived
        .iter(world)
        .filter_map(|(entity, name)| Some((*stand_ins.get(name)?, entity)))
        .collect();
    for (stand_in, entity) in adoptions {
        adopt(world, stand_in, entity);
    }
    let now = time::uptime(world);
    let me = session::my_id(world);
    let back = played.iter(world).any(|played| Some(played.client) == me);
    let over = match world.resource::<Switching>().0 {
        Some(since) => back || now - since > ARRIVAL_PATIENCE,
        None => true,
    };
    if over {
        world.resource_mut::<Switching>().0 = None;
        forget_lingering(world);
    }
}

/// Moves onto `entity` everything the client made of `stand_in` that `entity` doesn't have of
/// its own, its children with it, and lets the stand-in go.
fn adopt(world: &mut World, stand_in: Entity, entity: Entity) {
    let own: Vec<ComponentId> = world.entity(entity).archetype().components().to_vec();
    let children: Vec<Entity> = world
        .get::<Children>(stand_in)
        .map(|children| children.iter().collect())
        .unwrap_or_default();
    world
        .entity_mut(stand_in)
        .clone_with_opt_out(entity, move |builder| {
            builder
                .move_components(true)
                .deny::<(Lingering, Children, ChildOf)>()
                .deny_by_ids(own);
        });
    for child in children {
        world.entity_mut(child).insert(ChildOf(entity));
    }
    world.despawn(stand_in);
}

fn forget_lingering(world: &mut World) {
    let gone: Vec<Entity> = world
        .query_filtered::<Entity, With<Lingering>>()
        .iter(world)
        .collect();
    for entity in gone {
        world.despawn(entity);
    }
}
