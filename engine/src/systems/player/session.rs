use bevy_app::{App, Plugin, PreUpdate, Update};
use bevy_ecs::prelude::*;
use bevy_ecs::schedule::common_conditions::resource_removed;
use bevy_replication::{ClientReplicationPlugin, ClientSystems, ServerLink};
use bevy_time::{Real, Time};

use super::{ClientId, PlayedBy, Welcome};
use crate::core::render::transition::WorldViewSystems;
use crate::core::render::world::clouds::CloudDrift;
use crate::core::time::{self, DayClock, ServerClock, WeatherClock};
use crate::systems::{identity, stat};

pub struct ClientSessionPlugin;

impl Plugin for ClientSessionPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ClientReplicationPlugin);
        crate::systems::protocol(app);
        app.add_plugins(identity::ClientIdentityPlugin);
        app.init_resource::<MyClient>()
            .init_resource::<ServerClock>()
            .init_resource::<DayClock>()
            .init_resource::<WeatherClock>()
            .init_resource::<MyCharacter>();
        app.add_systems(Update, record_welcome);
        app.add_systems(
            PreUpdate,
            (
                track_character
                    .in_set(WorldViewSystems)
                    .after(ClientSystems::Receive),
                forget_me.run_if(resource_removed::<ServerLink>),
            ),
        );
    }
}

#[derive(Resource, Default)]
pub struct MyClient(pub Option<ClientId>);

/// The character this client plays and sees the world through, once it stands in the world.
#[derive(Resource, Default)]
pub struct MyCharacter(pub Option<Entity>);

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
    time: Res<Time<Real>>,
    mut me: ResMut<MyClient>,
    mut clock: ResMut<ServerClock>,
    mut day: ResMut<DayClock>,
    mut weather: ResMut<WeatherClock>,
    mut clouds: Option<ResMut<CloudDrift>>,
) {
    for welcome in welcomes.read() {
        me.0 = Some(welcome.id);
        clock.record(welcome.now, time::real_uptime(&time));
        day.0 = welcome.day;
        weather.0 = welcome.weather;
        if let Some(clouds) = clouds.as_mut() {
            **clouds = CloudDrift::told(welcome.clouds.to_array().into());
        }
    }
}

fn track_character(
    me: Res<MyClient>,
    characters: Query<(Entity, &PlayedBy)>,
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

fn forget_me(mut me: ResMut<MyClient>) {
    me.0 = None;
}
