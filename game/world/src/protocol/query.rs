use bevy_ecs::prelude::*;

use crate::content::area;
use crate::core::math::Pos;
use crate::core::tiling::{TilePos, Tiles};
use crate::protocol::session;
use crate::protocol::{Actor, AreaTag, Hitbox, Position, Vitals};

pub fn walkable(world: &World, tile: Pos<Tiles>) -> bool {
    session::me(world)
        .and_then(|me| me.get::<AreaTag>())
        .map(|tag| tag.area)
        .and_then(|id| area::areas().get(id.index()))
        .is_some_and(|area| area.grid.walkable(tile))
}

pub fn enemy_at(world: &mut World, point: Pos<Tiles>) -> Option<Entity> {
    let me = session::me(world).map(|entity| entity.id());
    let mut actors =
        world.query_filtered::<(Entity, &Position, &Hitbox, Option<&Vitals>), With<Actor>>();
    actors.iter(world).find_map(|(entity, at, hitbox, vitals)| {
        if Some(entity) == me || vitals.is_some_and(Vitals::is_dead) {
            return None;
        }
        at.pos.hitbox(hitbox.size).contains(point).then_some(entity)
    })
}
