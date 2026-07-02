use bevy_ecs::prelude::*;
use bevy_terminal::TerminalInbox;

use crate::data::terminal::Id;
use crate::systems::actor::Name;
use crate::systems::player::{self, ClientId};

pub fn rebroadcast(world: &mut World) {
    let entries = world.resource::<TerminalInbox<Id>>().0.clone();
    for entry in entries.iter().filter(|entry| !entry.consumed()) {
        let name = player::conn_player(world, entry.conn)
            .and_then(|player| world.get::<Name>(player))
            .map(|name| name.name.clone())
            .or_else(|| {
                world
                    .get::<ClientId>(entry.conn)
                    .map(|client| format!("player {}", client.0))
            })
            .unwrap_or_else(|| "player".to_owned());
        bevy_terminal::broadcast(world, entry.terminal, format!("{name}: {}", entry.text));
    }
}
