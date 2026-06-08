use rift::{Builder, ClientId, Ctx, Entity, Map, Wire, Zone};

use crate::core::area::AreaId;
use crate::core::identity::Identity;
use crate::core::math::{Direction, Millis, PlaybackRate, Pos, Tiles, TilesPerSec};
use crate::core::protocol::{
    ACTION_IDLE, Actor, AreaTag, Inventory, Name, Owner, Position, Rgba, Spectate, Vitals, Xp,
    is_dead, set_action,
};
use crate::core::{actors, area};
use crate::features::combat::Stats;
use crate::features::movement::{Speed, forget};
use crate::features::spectate::Spectators;

const PLAYER_MAX_HEALTH: f32 = 30.0;
const PLAYER_SPEED: TilesPerSec = TilesPerSec(Tiles(4.0));
const PLAYER_DAMAGE: f32 = 6.0;
const PLAYER_ATTACK_SPEED: PlaybackRate = PlaybackRate(1.2);
const PLAYER_ATTACK_DELAY: Millis = Millis(200.0);
const PLAYER_RANGE: Tiles = Tiles(1.5);
const PLAYER_TINT: Rgba = Rgba(0xFFFF_FFFF);
const PLAYER_MODEL: &str = "adventurer";

#[derive(Wire, Clone, Debug, PartialEq)]
pub struct JoinRequest {}

#[derive(Wire, Clone, Debug, PartialEq)]
pub struct RespawnRequest {}

pub struct Players(pub Map<ClientId, Entity>);

pub fn feature(b: &mut Builder) {
    b.start(|ctx| ctx.res.insert(Players(Map::default())));
    b.intent(join);
    b.disconnect(despawn);
    b.migrate(migrated);
    b.intent(respawn);
}

pub(crate) fn zone(ctx: &Ctx) -> AreaId {
    ctx.res
        .get::<Zone>()
        .map_or_else(area::spawn_zone, |z| AreaId(z.0))
}

fn join(ctx: &mut Ctx) {
    let zone = zone(ctx);
    let spawn = area::areas()[zone.0 as usize].spawn;
    for (client, _) in ctx.server.drain_events::<JoinRequest>() {
        let playing = ctx
            .res
            .get::<Players>()
            .is_some_and(|p| p.0.contains_key(&client));
        let spectating = ctx
            .res
            .get::<Spectators>()
            .is_some_and(|s| s.0.contains_key(&client));
        if playing || spectating {
            continue;
        }
        let name = ctx
            .server
            .session::<Identity>(client)
            .map_or_else(|| format!("player {}", client.0), |id| id.name.clone());
        spawn_player(ctx, client, zone, spawn, PLAYER_MAX_HEALTH, name);
    }
}

fn migrated(ctx: &mut Ctx, client: ClientId, entity: Entity) {
    if ctx.server.world.has::<Spectate>(entity) {
        return;
    }
    if let Some(players) = ctx.res.get_mut::<Players>() {
        players.0.insert(client, entity);
    }
}

fn spawn_player(
    ctx: &mut Ctx,
    client: ClientId,
    zone: AreaId,
    at: Pos<Tiles>,
    health: f32,
    name: String,
) {
    let entity = {
        let world = &mut ctx.server.world;
        let model = actors::model_index(PLAYER_MODEL).expect("the player model exists");
        let entity = world.spawn();
        world.insert(entity, Position { pos: at });
        world.insert(entity, Name { name });
        world.insert(
            entity,
            Actor {
                color: PLAYER_TINT,
                dir: Direction::S as u8,
                action: ACTION_IDLE,
                model,
                attack_rate: PLAYER_ATTACK_SPEED,
            },
        );
        world.insert(entity, actors::model_hitbox(model));
        world.insert(
            entity,
            Vitals {
                health,
                max: PLAYER_MAX_HEALTH,
            },
        );
        world.insert(entity, AreaTag { area: zone });
        world.insert(entity, Owner { client });
        world.insert(entity, Inventory { items: Vec::new() });
        world.insert(entity, Xp { amount: 0 });
        world.insert(
            entity,
            Stats {
                damage: PLAYER_DAMAGE,
                attack_speed: PLAYER_ATTACK_SPEED,
                attack_delay: PLAYER_ATTACK_DELAY,
                range: PLAYER_RANGE,
            },
        );
        world.insert(
            entity,
            Speed {
                value: PLAYER_SPEED,
            },
        );
        entity
    };
    if let Some(players) = ctx.res.get_mut::<Players>() {
        players.0.insert(client, entity);
    }
}

fn despawn(ctx: &mut Ctx, client: ClientId) {
    if let Some(entity) = ctx
        .res
        .get_mut::<Players>()
        .and_then(|p| p.0.remove(&client))
    {
        ctx.server.world.despawn(entity);
    }
}

fn respawn(ctx: &mut Ctx) {
    let zone = zone(ctx);
    let spawn = area::areas()[zone.0 as usize].spawn;
    for (client, _req) in ctx.server.drain_events::<RespawnRequest>() {
        let Some(&entity) = ctx.res.get::<Players>().and_then(|p| p.0.get(&client)) else {
            continue;
        };
        let world = &mut ctx.server.world;
        if !is_dead(world, entity) {
            continue;
        }
        world.modify::<Vitals>(entity, |v| v.health = v.max);
        world.modify::<Position>(entity, |p| p.pos = spawn);
        world.modify::<AreaTag>(entity, |tag| tag.area = zone);
        set_action(world, entity, ACTION_IDLE);
        forget(world, entity);
    }
}
