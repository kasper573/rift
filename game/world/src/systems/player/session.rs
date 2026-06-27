use bevy_app::{App, Plugin, Update};
use bevy_ecs::message::MessageReader;
use bevy_ecs::prelude::*;
use bevy_ecs::world::EntityRef;
use bevy_replicon::prelude::{AuthMethod, ClientState, RepliconPlugins, RepliconSharedPlugin};
use bevy_state::prelude::OnEnter;

use super::{ClientId, JoinRequest, Owner, RespawnRequest, Welcome};
use crate::core::math::Pos;
use crate::core::tiling::Tiles;
use crate::systems::area::{self, AreaTag};
use crate::systems::combat::AttackRequest;
use crate::systems::equipment::{Slot, UnequipRequest};
use crate::systems::item::{DropItemRequest, PickupRequest, UseItemRequest};
use crate::systems::movement::{MoveRequest, MoveToPortal};
use crate::systems::spectate::SpectateRequest;

pub struct ClientSessionPlugin;

impl Plugin for ClientSessionPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(
            bevy_app::PluginGroup::build(RepliconPlugins).set(RepliconSharedPlugin {
                auth_method: AuthMethod::None,
            }),
        );
        crate::systems::protocol(app);
        app.init_resource::<MyClient>();
        app.add_systems(Update, record_welcome);
        app.add_systems(OnEnter(ClientState::Disconnected), forget_me);
    }
}

#[derive(Resource, Default)]
pub struct MyClient(pub Option<ClientId>);

pub fn my_id(world: &World) -> Option<ClientId> {
    world.resource::<MyClient>().0
}

pub fn me(world: &World) -> Option<EntityRef<'_>> {
    let mine = my_id(world)?;
    world.iter_entities().find(|entity| {
        entity
            .get::<Owner>()
            .is_some_and(|owner| owner.client == mine)
    })
}

pub fn is_dead(world: &World) -> bool {
    me(world).is_some_and(|entity| crate::systems::stat::is_dead(world, entity.id()))
}

pub fn join(world: &mut World) {
    world.write_message(JoinRequest);
}

pub fn spectate(world: &mut World, watch: Option<ClientId>) {
    world.write_message(SpectateRequest { watch });
}

pub fn attack(world: &mut World, target: Entity) {
    world.write_message(AttackRequest { target });
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

pub fn unequip(world: &mut World, slot: Slot) {
    world.write_message(UnequipRequest { slot });
}

pub fn move_to(world: &mut World, pos: Pos<Tiles>) {
    let portal = me(world)
        .and_then(|entity| entity.get::<AreaTag>())
        .and_then(|tag| area::get(tag.area))
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

fn forget_me(mut me: ResMut<MyClient>) {
    me.0 = None;
}
