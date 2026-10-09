use bevy_app::{App, Plugin, PreUpdate, Update};
use bevy_ecs::message::MessageReader;
use bevy_ecs::prelude::*;
use bevy_ecs::world::EntityRef;
use bevy_replicon::client::ClientSystems;
use bevy_replicon::prelude::{AuthMethod, ClientState, RepliconPlugins, RepliconSharedPlugin};
use bevy_state::prelude::OnEnter;

use super::{ClientId, JoinRequest, Owner, RespawnRequest, Welcome};
use crate::core::math::Pos;
use crate::core::render::transition::WorldViewSystems;
use crate::core::tiling::Tiles;
use crate::systems::area;
use crate::systems::combat::AttackRequest;
use crate::systems::equipment::{EquipmentSlot, UnequipRequest};
use crate::systems::interact::InteractRequest;
use crate::systems::item::{DropItemRequest, PickupRequest, UseItemRequest};
use crate::systems::movement::{MoveRequest, MoveToPortal};
use crate::systems::spectate::{SpectateRequest, Spectating};

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
            .init_resource::<SpectateStatus>()
            .init_resource::<Viewpoint>();
        app.add_systems(Update, record_welcome);
        app.add_systems(
            PreUpdate,
            (record_spectating, track_viewpoint.in_set(WorldViewSystems))
                .chain()
                .after(ClientSystems::Receive),
        );
        app.add_systems(OnEnter(ClientState::Disconnected), forget_me);
    }
}

#[derive(Resource, Default)]
pub struct MyClient(pub Option<ClientId>);

/// What the server last told this client about whom it spectates; `None` until it has.
#[derive(Resource, Default)]
pub struct SpectateStatus(pub Option<Spectating>);

/// The character this client sees the world through: its own, or the one it spectates. There is no
/// other way to look at the world.
#[derive(Resource, Default)]
pub struct Viewpoint(pub Option<Entity>);

pub fn my_id(world: &World) -> Option<ClientId> {
    world.resource::<MyClient>().0
}

pub fn my_character(world: &World) -> Option<EntityRef<'_>> {
    let mine = my_id(world)?;
    world.iter_entities().find(|entity| {
        entity
            .get::<Owner>()
            .is_some_and(|owner| owner.client == mine)
    })
}

pub fn is_dead(world: &World) -> bool {
    my_character(world).is_some_and(|entity| crate::systems::stat::is_dead(world, entity.id()))
}

pub fn is_locked(world: &World) -> bool {
    my_character(world).is_some_and(|entity| super::commands_locked(world, entity.id()))
}

pub fn is_alive(world: &World) -> bool {
    my_character(world).is_some_and(|entity| !crate::systems::stat::is_dead(world, entity.id()))
}

pub fn followed(world: &World) -> Option<ClientId> {
    followed_client(
        world.resource::<MyClient>(),
        world.resource::<SpectateStatus>(),
    )
}

pub fn join(world: &mut World) {
    world.write_message(JoinRequest);
}

pub fn spectate(world: &mut World, request: SpectateRequest) {
    world.write_message(request);
}

pub fn attack(world: &mut World, target: Entity) {
    world.write_message(AttackRequest { target });
}

pub fn interact(world: &mut World, target: Entity) {
    world.write_message(InteractRequest { target });
}

pub fn respawn(world: &mut World) {
    world.write_message(RespawnRequest);
}

pub fn use_item(world: &mut World, slot: u32) {
    world.write_message(UseItemRequest { slot });
}

pub fn drop_item(world: &mut World, slot: u32) {
    world.write_message(DropItemRequest { slot });
}

pub fn pickup(world: &mut World, target: Entity) {
    world.write_message(PickupRequest { target });
}

pub fn unequip(world: &mut World, slot: EquipmentSlot) {
    world.write_message(UnequipRequest { slot });
}

pub fn move_to(world: &mut World, pos: Pos<Tiles>) {
    let portal = my_character(world)
        .map(|entity| entity.id())
        .and_then(|entity| area::of(world, entity))
        .and_then(|area| {
            area.portals
                .iter()
                .position(|portal| portal.rect.contains(pos))
        });
    match portal {
        Some(index) => {
            world.write_message(MoveToPortal {
                pos,
                portal: index as u32,
            });
        }
        None => {
            world.write_message(MoveRequest { pos });
        }
    }
}

fn record_welcome(mut welcomes: MessageReader<Welcome>, mut me: ResMut<MyClient>) {
    for welcome in welcomes.read() {
        me.0 = Some(welcome.id);
    }
}

fn record_spectating(mut told: MessageReader<Spectating>, mut status: ResMut<SpectateStatus>) {
    if let Some(&latest) = told.read().last() {
        status.0 = Some(latest);
    }
}

fn followed_client(me: &MyClient, status: &SpectateStatus) -> Option<ClientId> {
    match status.0 {
        Some(Spectating::Player(watched)) => Some(watched.player),
        Some(Spectating::Nobody) => None,
        None => me.0,
    }
}

fn track_viewpoint(
    me: Res<MyClient>,
    status: Res<SpectateStatus>,
    characters: Query<(Entity, &Owner)>,
    mut viewpoint: ResMut<Viewpoint>,
) {
    let followed = followed_client(&me, &status);
    let character = followed.and_then(|client| {
        characters
            .iter()
            .find(|(_, owner)| owner.client == client)
            .map(|(character, _)| character)
    });
    if viewpoint.0 != character {
        viewpoint.0 = character;
    }
}

fn forget_me(mut me: ResMut<MyClient>, mut status: ResMut<SpectateStatus>) {
    me.0 = None;
    status.0 = None;
}
