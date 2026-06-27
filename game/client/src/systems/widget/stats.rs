use bevy::prelude::*;
use ui::text_colored;
use world::systems::job;
use world::systems::player::session;
use world::systems::stat;

use super::Window;

#[derive(Component, Default, Clone)]
pub(super) struct StatsText;

pub struct StatsWindow;

impl Window for StatsWindow {
    fn title(&self) -> &'static str {
        "Stats"
    }
    fn toggle(&self) -> KeyCode {
        KeyCode::KeyK
    }
    fn keybind(&self) -> &'static str {
        "K"
    }
    fn icon(&self) -> &'static str {
        "icons/misc/book.png"
    }
    fn order(&self) -> u32 {
        2
    }
    fn content(&self) -> Box<dyn Scene> {
        content()
    }
    fn sync(&self, world: &mut World) {
        sync_stats(world)
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
    for stat in &stats.0 {
        lines.push(format!("{}: {:.1}", stat.label(), stat.value()));
    }
    lines.join("\n")
}
