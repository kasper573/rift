//! The client's view of the replicated world: who "me" is, the intents the player issues (join, move,
//! attack, …), and the spatial queries (`walkable`, `enemy_at`) the input gestures read. The server's
//! authoritative reaction to these lives in the `server` crate.

use bevy::app::{App, Plugin, Update};
use bevy::ecs::message::MessageReader;
use bevy::ecs::prelude::*;
use bevy::ecs::world::EntityRef;
use bevy::state::prelude::OnEnter;
use bevy_replicon::prelude::{AuthMethod, ClientState, RepliconPlugins, RepliconSharedPlugin};
use world::actor::{Actor, Hitbox};
use world::area::{self, AreaTag};
use world::combat::{AttackRequest, Vitals};
use world::core::math::Pos;
use world::core::tiling::{TilePos, Tiles};
use world::items::UseItemRequest;
use world::movement::{MoveRequest, MoveToPortal, Position};
use world::player::{ClientId, JoinRequest, Owner, RespawnRequest, Welcome};
use world::spectate::SpectateRequest;

pub struct ClientSessionPlugin;

impl Plugin for ClientSessionPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(
            bevy::app::PluginGroup::build(RepliconPlugins).set(RepliconSharedPlugin {
                auth_method: AuthMethod::None,
            }),
        );
        world::protocol(app);
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
    me(world).is_some_and(|entity| world::combat::is_dead(world, entity.id()))
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

/// Whether the local player can step onto `tile` in its current area — client-side path validation.
pub fn walkable(world: &World, tile: Pos<Tiles>) -> bool {
    me(world)
        .and_then(|me| me.get::<AreaTag>())
        .map(|tag| tag.area)
        .and_then(|id| area::areas().get(id.index()))
        .is_some_and(|area| area.grid.walkable(tile))
}

/// The living enemy (not the local player) whose hitbox covers `point` — the click's attack target.
pub fn enemy_at(world: &mut World, point: Pos<Tiles>) -> Option<Entity> {
    let me = me(world).map(|entity| entity.id());
    let mut actors =
        world.query_filtered::<(Entity, &Position, &Hitbox, Option<&Vitals>), With<Actor>>();
    actors.iter(world).find_map(|(entity, at, hitbox, vitals)| {
        if Some(entity) == me || vitals.is_some_and(Vitals::is_dead) {
            return None;
        }
        at.pos.hitbox(hitbox.size).contains(point).then_some(entity)
    })
}

fn record_welcome(mut welcomes: MessageReader<Welcome>, mut me: ResMut<MyClient>) {
    for welcome in welcomes.read() {
        me.0 = Some(welcome.id);
    }
}

/// A dropped link makes "me" unknown again, so a reconnect waits for a fresh [`Welcome`] before
/// announcing — exactly like a first connection — rather than acting on the stale id.
fn forget_me(mut me: ResMut<MyClient>) {
    me.0 = None;
}
