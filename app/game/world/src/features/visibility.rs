use rift::{Builder, ClientId, Entity, Set, View};

use crate::core::math::Tiles;
use crate::core::protocol::{AreaTag, Position, Spectate, position};
use crate::features::player::Players;

pub const VIEW_DISTANCE: Tiles = Tiles(24.0);

pub fn feature(b: &mut Builder) {
    b.see(see);
}

fn see(view: &View, client: ClientId, visible: &mut Set<Entity>) {
    let Some(&player) = view.res.get::<Players>().and_then(|p| p.0.get(&client)) else {
        return;
    };
    visible.insert(player);
    see_around(view, player, visible);
}

// Spectator anchors stay invisible to everyone; a spectator sees its own through its own hook.
pub(crate) fn see_around(view: &View, around: Entity, visible: &mut Set<Entity>) {
    let (Some(area), Some(at)) = (
        view.world.get::<AreaTag>(around).map(|tag| tag.area),
        position(view.world, around),
    ) else {
        return;
    };
    for (entity, p) in view.world.iter::<Position>() {
        if view.world.get::<AreaTag>(entity).map(|tag| tag.area) == Some(area)
            && at.distance(p.pos) <= VIEW_DISTANCE.0
            && !view.world.has::<Spectate>(entity)
        {
            visible.insert(entity);
        }
    }
}
