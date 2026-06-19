use bevy_app::{App, Plugin, Update};
use bevy_ecs::message::MessageReader;
use bevy_ecs::prelude::*;
use bevy_ecs::world::EntityRef;
use bevy_replicon::prelude::{AuthMethod, RepliconPlugins, RepliconSharedPlugin};

use crate::area;
use crate::math::{Pos, Tiles};
use crate::protocol::{
    self, AreaTag, AttackRequest, ClientId, JoinRequest, MoveRequest, MoveToPortal, Owner,
    RespawnRequest, SpectateRequest, UseItemRequest, Welcome,
};

/// Registers replicon's client plugins and the shared protocol, and records the [`ClientId`] the
/// server greets this session with. The caller adds a backend (`RepliconRenetPlugins` + a
/// `RenetClient`/transport) and a `StatesPlugin`, then ticks the app.
pub struct ClientSessionPlugin;

impl Plugin for ClientSessionPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(
            bevy_app::PluginGroup::build(RepliconPlugins).set(RepliconSharedPlugin {
                auth_method: AuthMethod::None,
            }),
        );
        protocol::protocol(app);
        app.init_resource::<MyClient>();
        app.add_systems(Update, record_welcome);
    }
}

/// The id the server greeted this session with; `None` until the welcome arrives.
#[derive(Resource, Default)]
pub struct MyClient(pub Option<ClientId>);

pub fn my_id(world: &World) -> Option<ClientId> {
    world.resource::<MyClient>().0
}

/// The character this client controls — the entity whose [`Owner`] carries our [`ClientId`]. It is
/// an ordinary character; read its components like any other: `me(world)?.get::<Vitals>()`.
pub fn me(world: &World) -> Option<EntityRef<'_>> {
    let mine = my_id(world)?;
    world.iter_entities().find(|entity| {
        entity
            .get::<Owner>()
            .is_some_and(|owner| owner.client == mine)
    })
}

pub fn is_dead(world: &World) -> bool {
    me(world).is_some_and(|entity| protocol::is_dead(world, entity.id()))
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

/// Walks to `pos`, picking the [`MoveToPortal`] intent when the point lands inside a portal of
/// the current area so the server warps instead of pathing there.
pub fn move_to(world: &mut World, pos: Pos<Tiles>) {
    let portal = me(world)
        .and_then(|entity| entity.get::<AreaTag>())
        .and_then(|tag| area::areas().get(tag.area.index()))
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
