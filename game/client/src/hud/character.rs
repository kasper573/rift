//! The character panel readout: the local player's name, health, and xp.

use crate::session;
use bevy::prelude::*;
use world::actor::Name;
use world::combat::Vitals;
use world::player::Xp;

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
    session::me(world).map_or_else(String::new, |me| {
        let (health, max) = me.get::<Vitals>().map_or((0.0, 0.0), |v| (v.health, v.max));
        let name = me
            .get::<Name>()
            .map_or_else(String::new, |n| n.name.clone());
        let xp = me.get::<Xp>().map_or(0, |x| x.amount);
        format!("{name}\n{health:.0} / {max:.0}\nxp {xp}")
    })
}
