use rift::{Builder, ClientId, Ctx, Entity, Map, Set, View, Wire};

use crate::core::area;
use crate::core::identity::Identity;
use crate::core::protocol::{AreaTag, Owner, Position, Spectate};
use crate::features::player::{Players, zone};
use crate::features::visibility;

pub const SPECTATE_ROLE: &str = "spectate";

#[derive(Wire, Clone, Debug, PartialEq)]
pub struct SpectateRequest {
    pub watch: Option<ClientId>,
}

// The anchor is a regular owned entity, so the cluster migrates it (and its client) across
// shards exactly like a player — spectating follows players through portals for free.
pub struct Spectators(pub Map<ClientId, Entity>);

pub fn feature(b: &mut Builder) {
    b.start(|ctx| ctx.res.insert(Spectators(Map::default())));
    b.intent(requests);
    b.system(follow);
    b.disconnect(despawn);
    b.migrate(migrated);
    b.see(see);
}

fn requests(ctx: &mut Ctx) {
    for (client, request) in ctx.server.drain_events::<SpectateRequest>() {
        if !allowed(ctx, client) {
            continue;
        }
        let anchor = ctx
            .res
            .get::<Spectators>()
            .and_then(|s| s.0.get(&client).copied());
        match anchor {
            Some(anchor) => ctx
                .server
                .world
                .modify::<Spectate>(anchor, |s| s.watch = request.watch),
            None => spawn_anchor(ctx, client, request.watch),
        }
    }
}

// No session means the server runs without an authenticator; anyone may spectate then.
fn allowed(ctx: &Ctx, client: ClientId) -> bool {
    let playing = ctx
        .res
        .get::<Players>()
        .is_some_and(|p| p.0.contains_key(&client));
    let entitled = match ctx.server.session::<Identity>(client) {
        Some(identity) => identity.has_role(SPECTATE_ROLE),
        None => true,
    };
    !playing && entitled
}

fn spawn_anchor(ctx: &mut Ctx, client: ClientId, watch: Option<ClientId>) {
    let zone = zone(ctx);
    let spawn = area::areas()[zone.0 as usize].spawn;
    let entity = {
        let world = &mut ctx.server.world;
        let entity = world.spawn();
        world.insert(entity, Position { pos: spawn });
        world.insert(entity, AreaTag { area: zone });
        world.insert(entity, Owner { client });
        world.insert(entity, Spectate { watch });
        entity
    };
    if let Some(spectators) = ctx.res.get_mut::<Spectators>() {
        spectators.0.insert(client, entity);
    }
}

fn follow(ctx: &mut Ctx) {
    let anchors: Vec<Entity> = ctx
        .res
        .get::<Spectators>()
        .map_or_else(Vec::new, |s| s.0.values().copied().collect());
    for anchor in anchors {
        let Some(Some(watch)) = ctx.server.world.get::<Spectate>(anchor).map(|s| s.watch) else {
            continue;
        };
        let target = ctx
            .res
            .get::<Players>()
            .and_then(|p| p.0.get(&watch).copied());
        let world = &mut ctx.server.world;
        let Some(player) = target else {
            world.modify::<Spectate>(anchor, |s| s.watch = None);
            continue;
        };
        let (Some(at), Some(area)) = (world.get::<Position>(player), world.get::<AreaTag>(player))
        else {
            continue;
        };
        if world.get::<Position>(anchor) != Some(at.clone()) {
            world.insert(anchor, at);
        }
        if world.get::<AreaTag>(anchor) != Some(area.clone()) {
            world.insert(anchor, area);
        }
    }
}

fn despawn(ctx: &mut Ctx, client: ClientId) {
    if let Some(entity) = ctx
        .res
        .get_mut::<Spectators>()
        .and_then(|s| s.0.remove(&client))
    {
        ctx.server.world.despawn(entity);
    }
}

fn migrated(ctx: &mut Ctx, client: ClientId, entity: Entity) {
    if ctx.server.world.has::<Spectate>(entity)
        && let Some(spectators) = ctx.res.get_mut::<Spectators>()
    {
        spectators.0.insert(client, entity);
    }
}

// All of the shard's players stay visible so the spectate UI can offer the full roster.
fn see(view: &View, client: ClientId, visible: &mut Set<Entity>) {
    let Some(&anchor) = view.res.get::<Spectators>().and_then(|s| s.0.get(&client)) else {
        return;
    };
    visible.insert(anchor);
    visibility::see_around(view, anchor, visible);
    if let Some(players) = view.res.get::<Players>() {
        visible.extend(players.0.values().copied());
    }
}
