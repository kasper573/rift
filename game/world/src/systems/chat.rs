use bevy_ecs::prelude::*;

use crate::systems::actor::Name;
use crate::systems::terminal::{self, TerminalInbox};

/// Any input no other integration consumed becomes a chat line on the terminal it was typed
/// into, visible to everyone with access to that terminal.
pub fn rebroadcast(world: &mut World) {
    let entries = world.resource::<TerminalInbox>().0.clone();
    for entry in entries.iter().filter(|entry| !entry.consumed()) {
        let name = entry
            .player
            .and_then(|player| world.get::<Name>(player))
            .map_or_else(
                || format!("player {}", entry.client.0),
                |name| name.name.clone(),
            );
        terminal::broadcast(world, entry.terminal, format!("{name}: {}", entry.text));
    }
}
