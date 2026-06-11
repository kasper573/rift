use bevy_app::{App, Plugin, Update};
use bevy_ecs::message::MessageReader;
use bevy_ecs::prelude::*;
use bevy_replicon::prelude::{AuthMethod, RepliconPlugins, RepliconSharedPlugin};

use crate::area::{self, AreaId};
use crate::math::{Pos, Tiles};
use crate::protocol::{
    self, Actor, AreaTag, AttackRequest, ClientId, Inventory, ItemId, JoinRequest, MoveRequest,
    MoveToPortal, Name, Owner, Position, RespawnRequest, Spectate, SpectateRequest, UseItemRequest,
    Vitals, Welcome, Xp,
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

/// Walks to `(x, y)`, picking the [`MoveToPortal`] intent when the point lands inside a portal of
/// the current area so the server warps instead of pathing there.
pub fn move_to(world: &mut World, x: f32, y: f32) {
    let portal = my_area(world).and_then(|area| {
        area::areas()
            .get(area.0 as usize)?
            .portals
            .iter()
            .position(|portal| portal.rect.contains(Pos::new(x, y)))
    });
    match portal {
        Some(index) => {
            world.write_message(MoveToPortal {
                pos: Pos::new(x, y),
                portal: index as u32,
            });
        }
        None => {
            world.write_message(MoveRequest {
                pos: Pos::new(x, y),
            });
        }
    }
}

pub fn my_entity(world: &World) -> Option<Entity> {
    let me = my_id(world)?;
    world
        .iter_entities()
        .find(|entity| {
            entity
                .get::<Owner>()
                .is_some_and(|owner| owner.client == me)
        })
        .map(|entity| entity.id())
}

pub fn my_position(world: &World) -> Option<Pos<Tiles>> {
    world.get::<Position>(my_entity(world)?).map(|p| p.pos)
}

pub fn my_vitals(world: &World) -> Option<(f32, f32)> {
    world
        .get::<Vitals>(my_entity(world)?)
        .map(|vitals| (vitals.health, vitals.max))
}

pub fn my_name(world: &World) -> Option<String> {
    world.get::<Name>(my_entity(world)?).map(|n| n.name.clone())
}

pub fn my_xp(world: &World) -> Option<u32> {
    world.get::<Xp>(my_entity(world)?).map(|xp| xp.amount)
}

pub fn my_inventory(world: &World) -> Vec<ItemId> {
    my_entity(world)
        .and_then(|entity| world.get::<Inventory>(entity))
        .map_or_else(Vec::new, |inventory| inventory.items.clone())
}

pub fn my_area(world: &World) -> Option<AreaId> {
    world.get::<AreaTag>(my_entity(world)?).map(|tag| tag.area)
}

pub fn is_dead(world: &World) -> bool {
    my_vitals(world).is_some_and(|(health, _)| health <= 0.0)
}

pub fn is_spectating(world: &World) -> bool {
    my_entity(world).is_some_and(|entity| world.get::<Spectate>(entity).is_some())
}

pub fn watching(world: &World) -> Option<ClientId> {
    world.get::<Spectate>(my_entity(world)?)?.watch
}

/// Every other player in view, by id and name, sorted by id.
pub fn players(world: &World) -> Vec<(ClientId, String)> {
    let me = my_id(world);
    let mut players: Vec<(ClientId, String)> = world
        .iter_entities()
        .filter_map(|entity| {
            let owner = entity.get::<Owner>()?;
            (Some(owner.client) != me && entity.contains::<Actor>()).then(|| {
                (
                    owner.client,
                    entity
                        .get::<Name>()
                        .map_or_else(String::new, |name| name.name.clone()),
                )
            })
        })
        .collect();
    players.sort_unstable_by_key(|(id, _)| *id);
    players
}

fn record_welcome(mut welcomes: MessageReader<Welcome>, mut me: ResMut<MyClient>) {
    for welcome in welcomes.read() {
        me.0 = Some(welcome.id);
    }
}
