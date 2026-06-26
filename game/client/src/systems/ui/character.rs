//! The character panel readout: the local player's name, health, and xp.

use bevy::prelude::*;
use world::systems::actor::Name;
use world::systems::player::Xp;
use world::systems::player::session;
use world::systems::stat;

#[derive(Component, Default, Clone)]
pub(super) struct CharacterText;

pub(super) fn sync_character(world: &mut World) {
    let text = character_text(world);
    let mut query = world.query_filtered::<&mut Text, With<CharacterText>>();
    for mut node in query.iter_mut(world) {
        node.0 = text.clone();
    }
}

fn character_text(world: &World) -> String {
    let Some(me) = session::me(world) else {
        return String::new();
    };
    let entity = me.id();
    let name = me
        .get::<Name>()
        .map_or_else(String::new, |n| n.name.clone());
    let xp = me.get::<Xp>().map_or(0, |x| x.amount);
    let health = stat::current_health(world, entity);
    let max = stat::max_health(world, entity);
    format!("{name}\n{health:.0} / {max:.0}\nxp {xp}")
}
