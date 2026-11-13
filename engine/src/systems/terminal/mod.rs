pub mod widget;

use std::collections::VecDeque;

use bevy::prelude::*;
use bevy_terminal::TerminalLine;

pub struct TerminalPlugin;

impl Plugin for TerminalPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TerminalLog>()
            .init_resource::<widget::TerminalView>()
            .add_systems(Update, (receive, widget::suggested));
    }
}

#[derive(Resource, Default)]
pub struct TerminalLog {
    lines: VecDeque<TerminalLine>,
}

impl TerminalLog {
    pub fn lines(&self) -> impl Iterator<Item = &TerminalLine> {
        self.lines.iter()
    }
}

const MAX_LINES: usize = 200;

fn receive(mut incoming: MessageReader<TerminalLine>, mut log: ResMut<TerminalLog>) {
    for line in incoming.read() {
        log.lines.push_back(line.clone());
        if log.lines.len() > MAX_LINES {
            log.lines.pop_front();
        }
    }
}
