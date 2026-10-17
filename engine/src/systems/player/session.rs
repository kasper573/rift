use bevy_app::{App, Plugin, PreUpdate, Update};
use bevy_ecs::prelude::*;
use bevy_replicon::client::ClientSystems;
use bevy_replicon::prelude::{AuthMethod, ClientState, RepliconPlugins, RepliconSharedPlugin};
use bevy_replicon::shared::server_entity_map::ServerEntityMap;
use bevy_state::prelude::{OnEnter, OnExit};

use super::{ClientId, Owner, Welcome};
use crate::core::render::transition::WorldViewSystems;
use crate::core::time::{Seconds, ServerClock, UnixMillis};
use crate::systems::stat;
use bevy_time::Time;

pub struct ClientSessionPlugin;

impl Plugin for ClientSessionPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(
            bevy_app::PluginGroup::build(RepliconPlugins).set(RepliconSharedPlugin {
                auth_method: AuthMethod::None,
            }),
        );
        crate::systems::protocol(app);
        app.init_resource::<MyClient>()
            .init_resource::<ServerClock>()
            .init_resource::<MyCharacter>();
        app.add_systems(Update, record_welcome);
        app.add_systems(
            PreUpdate,
            track_character
                .in_set(WorldViewSystems)
                .after(ClientSystems::Receive),
        );
        app.add_systems(OnEnter(ClientState::Disconnected), forget_me);
        app.add_systems(
            OnExit(ClientState::Connected),
            drop_replicated.before(ClientSystems::Reset),
        );
    }
}

#[derive(Resource, Default)]
pub struct MyClient(pub Option<ClientId>);

/// The character this client plays and sees the world through, once it stands in the world.
#[derive(Resource, Default)]
pub struct MyCharacter(pub Option<Entity>);

pub fn server_now(world: &World) -> Option<UnixMillis> {
    let uptime = Seconds(world.resource::<Time>().elapsed_secs());
    world.resource::<ServerClock>().now(uptime)
}

pub fn my_id(world: &World) -> Option<ClientId> {
    world.resource::<MyClient>().0
}

pub fn me(world: &World) -> Option<Entity> {
    world.resource::<MyCharacter>().0
}

pub fn my_character(world: &World) -> Option<EntityRef<'_>> {
    me(world).and_then(|character| world.get_entity(character).ok())
}

pub fn is_dead(world: &World) -> bool {
    my_character(world).is_some_and(|entity| stat::is_dead(world, entity.id()))
}

pub fn is_locked(world: &World) -> bool {
    my_character(world).is_some_and(|entity| super::commands_locked(world, entity.id()))
}

pub fn is_alive(world: &World) -> bool {
    my_character(world).is_some_and(|entity| !stat::is_dead(world, entity.id()))
}

fn record_welcome(
    mut welcomes: MessageReader<Welcome>,
    time: Res<Time>,
    mut me: ResMut<MyClient>,
    mut clock: ResMut<ServerClock>,
) {
    for welcome in welcomes.read() {
        me.0 = Some(welcome.id);
        clock.record(welcome.now, Seconds(time.elapsed_secs()));
    }
}

fn track_character(
    me: Res<MyClient>,
    characters: Query<(Entity, &Owner)>,
    mut mine: ResMut<MyCharacter>,
) {
    let my_character = me.0.and_then(|client| {
        characters
            .iter()
            .find(|(_, owner)| owner.client == client)
            .map(|(character, _)| character)
    });
    if mine.0 != my_character {
        mine.0 = my_character;
    }
}

/// A connection's world ends with it: the next server (the same one restarted, or another after a
/// deploy) numbers its entities afresh, so nothing of the old one may stay to be mistaken for new.
fn drop_replicated(world: &mut World) {
    let entities: Vec<Entity> = world
        .resource::<ServerEntityMap>()
        .to_client()
        .values()
        .copied()
        .collect();
    for entity in entities {
        if let Ok(entity) = world.get_entity_mut(entity) {
            entity.despawn();
        }
    }
}

fn forget_me(mut me: ResMut<MyClient>) {
    me.0 = None;
}
