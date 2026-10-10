use crate::systems::interface::InterfaceIcon;
use crate::systems::job;
use crate::systems::player::session;
use crate::systems::stat;
use bevy::prelude::*;
use ui::text_colored;

use crate::systems::hud::{HudAudience, Window};
use crate::systems::input::map::InputAction;

#[derive(Component, Default, Clone)]
pub(super) struct StatsText;

pub struct StatsWindow;

impl Window for StatsWindow {
    fn audience(&self) -> HudAudience {
        HudAudience::Players
    }
    fn title(&self) -> &'static str {
        "Stats"
    }
    fn toggle(&self) -> InputAction {
        InputAction::ToggleStats
    }
    fn icon(&self) -> InterfaceIcon {
        InterfaceIcon::Stats
    }
    fn order(&self) -> u32 {
        2
    }
    fn contents(&self, _: &World) -> Vec<ui::WindowContent> {
        crate::systems::hud::single_tab(self.title(), ui::scrolled(content()))
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
    let Some(me) = session::my_character(world) else {
        return String::new();
    };
    let entity = me.id();
    let stats = stat::effective_all(world, entity);
    let mut lines = vec![format!("Level {}", job::level(world, entity))];
    for stat in &stats.0 {
        lines.push(format!("{}: {:.1}", stat.label(), stat.value));
    }
    lines.join("\n")
}
