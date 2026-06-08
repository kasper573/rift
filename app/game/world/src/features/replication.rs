use rift::{Builder, Entity, World};

use crate::core::protocol::{
    Actor, AreaTag, Hitbox, Inventory, Name, Owner, Position, Spectate, Vitals, Xp,
};

// shard_by/owned_by is the entire surface the game exposes to sharding: rift migrates an entity
// with all of its state whenever its area changes, with no migration code in the game.
pub fn feature(b: &mut Builder) {
    b.replicate::<Position>();
    b.replicate::<Actor>();
    b.replicate::<Hitbox>();
    b.replicate::<Vitals>();
    b.replicate::<AreaTag>();
    b.replicate::<Owner>();
    b.replicate::<Name>();
    b.replicate::<Spectate>();
    b.replicate::<Xp>();
    b.replicate_to_owner::<Inventory>();
    b.shard_by(|world: &World, entity: Entity| world.get::<AreaTag>(entity).map(|tag| tag.area.0));
    b.owned_by(|world: &World, entity: Entity| {
        world.get::<Owner>(entity).map(|owner| owner.client)
    });
}
