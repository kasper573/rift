//! The stats pane: a readout of the local player's level and effective stats, recomputed on the
//! client from the same base and effects the server aggregates, so the two always agree.

use bevy::prelude::*;
use world::systems::job;
use world::systems::player::session;
use world::systems::stat::{self, Stat, StatKind};

#[derive(Component, Default, Clone)]
pub(super) struct StatsText;

pub(super) fn sync_stats(world: &mut World) {
    let text = stats_text(world);
    let mut query = world.query_filtered::<&mut Text, With<StatsText>>();
    for mut node in query.iter_mut(world) {
        node.0 = text.clone();
    }
}

fn stats_text(world: &World) -> String {
    let Some(me) = session::me(world) else {
        return String::new();
    };
    let entity = me.id();
    let stats = stat::effective_all(world, entity);
    let mut lines = vec![format!("Level {}", job::level(world, entity))];
    for kind in StatKind::all() {
        lines.push(format!("{}: {:.1}", kind.label(), stats.get(kind)));
    }
    lines.join("\n")
}
