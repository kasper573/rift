//! Wires the world's client session and, once the netcode connection is welcomed, announces the mode
//! the connection was opened in (join or spectate). The generic transport lives in `crate::core::net`.

use bevy::prelude::*;
use world::systems::player::session::{self, ClientSessionPlugin};

use crate::core::net::Announce;

pub struct SessionPlugin;

impl Plugin for SessionPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ClientSessionPlugin)
            .add_systems(Update, announce);
    }
}

fn announce(world: &mut World) {
    if world.get_resource::<Announce>().is_none() || session::my_id(world).is_none() {
        return;
    }
    let spectate = world.resource::<Announce>().spectate;
    info!(
        "connection welcomed; announcing {}",
        if spectate { "spectate" } else { "join" }
    );
    if spectate {
        session::spectate(world, None);
    } else {
        session::join(world);
    }
    world.remove_resource::<Announce>();
}
