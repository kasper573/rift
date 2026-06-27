use bevy::prelude::*;
use ui::text_colored;
use world::systems::job;
use world::systems::player::session;
use world::systems::stat;

use super::WindowDef;

#[derive(Component, Default, Clone)]
pub(super) struct StatsText;

inventory::submit! {
    WindowDef {
        id: "Stats",
        title: "Stats",
        toggle: KeyCode::KeyK,
        keybind: "K",
        icon: "icons/misc/book.png",
        order: 2,
        content,
        sync: sync_stats,
    }
}

fn content() -> Box<dyn Scene> {
    Box::new(bsn! {
        Node { width: Val::Percent(100.0) }
        Children [ ( {text_colored(String::new(), Color::WHITE)} StatsText ) ]
    })
}

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
    for kind in stat::all() {
        lines.push(format!("{}: {:.1}", kind.label(), stats.get(kind)));
    }
    lines.join("\n")
}
